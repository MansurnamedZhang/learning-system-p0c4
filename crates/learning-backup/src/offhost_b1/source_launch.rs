//! Default-off fixed original Root producer and strict control/C11 primitives.
//!
//! The zero-argument burned entry admits its own actual native/code/metadata
//! stack and retains the original job, children, exec, streams and generations.
//! DATA parsing and ordinary component tests grant no installed-operation credit.
use crate::{BackupError, registry};
use learning_assets::{
    InstalledSnapshotBinding, backup_fs::BackupDir, root_journal::SharedJournalBudget,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, Metadata},
    io::{Seek, SeekFrom},
    os::unix::{fs::MetadataExt, io::AsRawFd, net::UnixStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::{Uuid, Variant, Version};

const RECORD_CAP: usize = 1024 * 1024;
const FRAME_CAP: usize = 4 * 1024 * 1024;
const CONTAINER_CAP: usize = 1024;
const ENTRY_CAP: usize = 100_000;
const GENESIS_NAME: &str = "00000000000000000001.frame";
const CONTROL_CAP: usize = 4096;
const CONTROL_FRAMES: u64 = 64;
const CONTROL_BYTES: u64 = 262_400;
const OBSERVATION_NAME: &str = "install-cutover-observation.json";

fn invalid() -> BackupError {
    BackupError::Invalid("B1 C11 original DATA mismatch")
}
fn text(value: &str, cap: usize) -> bool {
    value.len() <= cap && !value.contains('\0')
}
fn path(value: &str) -> bool {
    text(value, 4096) && registry::canonical_path(value)
}
fn v4(value: Uuid) -> bool {
    value.get_variant() == Variant::RFC4122 && value.get_version() == Some(Version::Random)
}
fn pay(meter: &SharedJournalBudget, bytes: usize, entries: usize) -> Result<(), BackupError> {
    root_runtime::scan_charge(
        meter,
        u64::try_from(bytes).map_err(|_| invalid())?,
        u64::try_from(entries).map_err(|_| invalid())?,
    )?;
    Ok(())
}
fn hash(raw: &[u8], meter: &SharedJournalBudget) -> Result<String, BackupError> {
    pay(meter, raw.len().checked_add(64).ok_or_else(invalid)?, 0)?;
    Ok(hex::encode(Sha256::digest(raw)))
}

// Charge a bounded lexical pass before typed deserialization allocates. The
// actual structural tokens prepay parsing entries; per-object schema caps below
// remain additional checks and never reset the original cumulative meter.
fn parse<T: DeserializeOwned + Serialize>(
    raw: &[u8],
    cap: usize,
    meter: &SharedJournalBudget,
) -> Result<T, BackupError> {
    if raw.is_empty() || raw.len() > cap {
        return Err(invalid());
    }
    pay(meter, raw.len(), 0)?;
    let (mut quoted, mut escaped, mut entries) = (false, false, 1_usize);
    for byte in raw {
        if quoted {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                quoted = false;
            }
        } else if *byte == b'"' {
            quoted = true;
        } else if matches!(*byte, b'{' | b'[' | b',' | b':') {
            entries = entries
                .checked_add(1)
                .filter(|n| *n <= ENTRY_CAP)
                .ok_or_else(invalid)?;
        }
    }
    // Covers the typed tree, canonical Value/string buffers and serialization
    // copies conservatively; it is paid before any of those allocations occur.
    pay(
        meter,
        raw.len().checked_mul(64).ok_or_else(invalid)?,
        entries,
    )?;
    registry::canonical_record(raw, cap as u64)
}

// The following values are DATA, including BrokerInstalled's correlation
// fields. Parsing them does not authenticate a channel or qualify a process.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BrokerInstalledData {
    schema_version: u8,
    kind: String,
    job_id: Uuid,
    root_cookie: String,
    sequence: u64,
    backup_id: Uuid,
    request_id: Uuid,
    binding_id: Uuid,
    epoch_id: Uuid,
    descriptor_sha256: String,
}
impl BrokerInstalledData {
    fn validate(&self) -> Result<(), BackupError> {
        if self.schema_version != 1
            || self.kind != "BrokerInstalled"
            || ![
                self.job_id,
                self.backup_id,
                self.request_id,
                self.binding_id,
                self.epoch_id,
            ]
            .into_iter()
            .all(v4)
            || !crate::valid_digest(&self.root_cookie)
            || !crate::valid_digest(&self.descriptor_sha256)
        {
            return Err(invalid());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct MountProjection {
    source: String,
    destination: String,
    rw: bool,
    kind: String,
    #[serde(deserialize_with = "required_nullable_string")]
    name: Option<String>,
}
fn required_nullable_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(d)
}
impl MountProjection {
    fn validate(&self) -> Result<(), BackupError> {
        if !path(&self.source)
            || !path(&self.destination)
            || !text(&self.kind, 64)
            || self.name.as_ref().is_some_and(|s| !text(s, 4096))
        {
            return Err(invalid());
        }
        Ok(())
    }
}

// Ordinary Option<Option<String>> loses absent versus JSON null. Preserve both
// exactly for the original raw Docker assets Mounts object.
#[derive(Default, PartialEq, Eq)]
enum ActualOptionalString {
    #[default]
    Missing,
    Present(Option<String>),
}
impl ActualOptionalString {
    fn missing(&self) -> bool {
        matches!(self, Self::Missing)
    }
    fn valid(&self, cap: usize) -> bool {
        match self {
            Self::Present(Some(s)) => text(s, cap),
            _ => true,
        }
    }
    fn normalized(&self) -> Option<&str> {
        match self {
            Self::Present(Some(s)) => Some(s),
            _ => None,
        }
    }
}
impl Serialize for ActualOptionalString {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Missing => s.serialize_none(),
            Self::Present(value) => value.serialize(s),
        }
    }
}
impl<'de> Deserialize<'de> for ActualOptionalString {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self::Present(Option::<String>::deserialize(d)?))
    }
}
#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ActualAssetsMount {
    #[serde(rename = "Type")]
    kind: String,
    #[serde(rename = "Source")]
    source: String,
    #[serde(rename = "Destination")]
    destination: String,
    #[serde(rename = "RW")]
    rw: bool,
    #[serde(
        rename = "Name",
        default,
        skip_serializing_if = "ActualOptionalString::missing"
    )]
    name: ActualOptionalString,
    #[serde(
        rename = "Driver",
        default,
        skip_serializing_if = "ActualOptionalString::missing"
    )]
    driver: ActualOptionalString,
    #[serde(
        rename = "Mode",
        default,
        skip_serializing_if = "ActualOptionalString::missing"
    )]
    mode: ActualOptionalString,
    #[serde(
        rename = "Propagation",
        default,
        skip_serializing_if = "ActualOptionalString::missing"
    )]
    propagation: ActualOptionalString,
}
impl ActualAssetsMount {
    fn validate(&self) -> Result<(), BackupError> {
        if !matches!(self.kind.as_str(), "bind" | "volume")
            || !path(&self.source)
            || !path(&self.destination)
            || self.rw
            || !self.name.valid(4096)
            || !self.driver.valid(128)
            || !self.mode.valid(256)
            || !self.propagation.valid(128)
        {
            return Err(invalid());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ContainerProjection {
    container_id: String,
    image_sha256: String,
    running: bool,
    process_id: u64,
    started_at: String,
    restart_count: u64,
    #[serde(deserialize_with = "required_nullable_string")]
    project: Option<String>,
    #[serde(deserialize_with = "required_nullable_string")]
    service: Option<String>,
    mounts: Vec<MountProjection>,
}
impl ContainerProjection {
    fn validate(&self) -> Result<(), BackupError> {
        if !crate::valid_digest(&self.container_id)
            || !self
                .image_sha256
                .strip_prefix("sha256:")
                .is_some_and(crate::valid_digest)
            || !text(&self.started_at, 128)
            || self.project.as_ref().is_some_and(|s| !text(s, 1024))
            || self.service.as_ref().is_some_and(|s| !text(s, 1024))
            || self.mounts.len() > ENTRY_CAP
        {
            return Err(invalid());
        }
        for mount in &self.mounts {
            mount.validate()?;
        }
        if self
            .mounts
            .windows(2)
            .any(|w| w[0].destination >= w[1].destination)
        {
            return Err(invalid());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct HostCutoverData {
    container_id: String,
    image_sha256: String,
    export_mount: String,
    journal_mount: String,
    assets_mount: ActualAssetsMount,
    legacy_producers: Vec<ContainerProjection>,
    complete_container_projection: Vec<ContainerProjection>,
    daemon_id: String,
    mountinfo_sha256: String,
    docker_cli_sha256: String,
}
impl HostCutoverData {
    fn validate(&self) -> Result<(), BackupError> {
        if !crate::valid_digest(&self.container_id)
            || !self
                .image_sha256
                .strip_prefix("sha256:")
                .is_some_and(crate::valid_digest)
            || !path(&self.export_mount)
            || !path(&self.journal_mount)
            || !text(&self.daemon_id, 128)
            || !crate::valid_digest(&self.mountinfo_sha256)
            || !crate::valid_digest(&self.docker_cli_sha256)
            || self.complete_container_projection.is_empty()
            || self.complete_container_projection.len() > CONTAINER_CAP
            || self.legacy_producers.len() > CONTAINER_CAP
        {
            return Err(invalid());
        }
        self.assets_mount.validate()?;
        for rows in [&self.complete_container_projection, &self.legacy_producers] {
            for row in rows {
                row.validate()?;
            }
            if rows
                .windows(2)
                .any(|w| w[0].container_id >= w[1].container_id)
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstallCutoverObservationData {
    schema_version: u8,
    capability: String,
    job_id: Uuid,
    backup_id: Uuid,
    phase: String,
    request_id: Uuid,
    binding_id: Uuid,
    epoch_id: Uuid,
    generation_binding_sha256: String,
    descriptor_sha256: String,
    epoch_header_sha256: String,
    genesis_frame_sha256: String,
    observed_unix_ms: u64,
    host_cutover: HostCutoverData,
}
impl InstallCutoverObservationData {
    fn read_data(raw: &[u8], meter: &SharedJournalBudget) -> Result<Self, BackupError> {
        let value: Self = parse(raw, RECORD_CAP, meter)?;
        if value.schema_version != 1
            || value.capability != "b1_install_cutover_observation_v1"
            || value.phase != "Install"
            || value.observed_unix_ms == 0
            || ![
                value.job_id,
                value.backup_id,
                value.request_id,
                value.binding_id,
                value.epoch_id,
            ]
            .into_iter()
            .all(v4)
            || [
                &value.generation_binding_sha256,
                &value.descriptor_sha256,
                &value.epoch_header_sha256,
                &value.genesis_frame_sha256,
            ]
            .into_iter()
            .any(|s| !crate::valid_digest(s))
        {
            return Err(invalid());
        }
        value.host_cutover.validate()?;
        Ok(value)
    }
    fn matches_notice(&self, notice: &BrokerInstalledData) -> Result<(), BackupError> {
        notice.validate()?;
        if self.job_id != notice.job_id
            || self.backup_id != notice.backup_id
            || self.request_id != notice.request_id
            || self.binding_id != notice.binding_id
            || self.epoch_id != notice.epoch_id
            || self.descriptor_sha256 != notice.descriptor_sha256
        {
            return Err(invalid());
        }
        Ok(())
    }
}

// Pure comparison inputs, not an admitted Catalog or Root owner. The future
// original Root job must derive these from its actual pinned/current records.
struct InstalledMountData {
    container_id: String,
    image_sha256: String,
    daemon_id: String,
    compose_project: String,
    assets_inside: String,
    mount_digest: String,
}
fn contains(parent: &str, child: &str) -> bool {
    parent == child
        || child
            .strip_prefix(parent)
            .is_some_and(|tail| tail.starts_with('/'))
}
fn current<'a>(
    host: &'a HostCutoverData,
    installed: &InstalledMountData,
) -> Result<&'a ContainerProjection, BackupError> {
    host.validate()?;
    if host.container_id != installed.container_id
        || host.image_sha256 != installed.image_sha256
        || host.daemon_id != installed.daemon_id
        || !path(&installed.assets_inside)
        || !crate::valid_digest(&installed.mount_digest)
    {
        return Err(invalid());
    }
    host.complete_container_projection
        .iter()
        .find(|r| r.container_id == host.container_id)
        .ok_or_else(invalid)
}
fn dedicated<'a>(
    row: &'a ContainerProjection,
    destination: &str,
) -> Result<&'a MountProjection, BackupError> {
    let mount = row
        .mounts
        .iter()
        .find(|m| m.destination == destination)
        .ok_or_else(invalid)?;
    if mount.kind != "volume"
        || !mount.rw
        || mount.name.as_ref().is_none_or(String::is_empty)
        || row
            .mounts
            .iter()
            .any(|m| m.destination != destination && contains(destination, &m.destination))
    {
        return Err(invalid());
    }
    Ok(mount)
}
fn check_projection(
    host: &HostCutoverData,
    installed: &InstalledMountData,
    meter: &SharedJournalBudget,
) -> Result<(), BackupError> {
    let own = current(host, installed)?;
    if !own.running
        || own.process_id == 0
        || own.image_sha256 != installed.image_sha256
        || own.project.as_deref() != Some(installed.compose_project.as_str())
        || own.service.as_deref() != Some("pg")
        || own.mounts.len() != 12
    {
        return Err(invalid());
    }
    let export = dedicated(own, InstalledSnapshotBinding::EXPORT_PATH)?;
    let companion = dedicated(own, InstalledSnapshotBinding::COMPANION_PATH)?;
    if export.name == companion.name
        || export.source != host.export_mount
        || companion.source != host.journal_mount
        || contains(&export.source, &companion.source)
        || contains(&companion.source, &export.source)
    {
        return Err(invalid());
    }
    let assets = own
        .mounts
        .iter()
        .filter(|m| contains(&m.destination, &installed.assets_inside))
        .max_by_key(|m| m.destination.len())
        .ok_or_else(invalid)?;
    if assets.rw
        || host.assets_mount.source != assets.source
        || host.assets_mount.destination != assets.destination
        || host.assets_mount.kind != assets.kind
        || host.assets_mount.name.normalized() != assets.name.as_deref()
    {
        return Err(invalid());
    }
    let mut legacy = host.legacy_producers.iter();
    for row in &host.complete_container_projection {
        pay(
            meter,
            0,
            row.mounts.len().checked_add(1).ok_or_else(invalid)?,
        )?;
        if row.container_id != own.container_id {
            if row.mounts.iter().any(|m| {
                m.rw && [&export.source, &companion.source]
                    .into_iter()
                    .any(|root| contains(&m.source, root) || contains(root, &m.source))
            }) {
                return Err(invalid());
            }
            if row.project.as_deref() == Some(installed.compose_project.as_str()) {
                if row.service.as_deref() != Some("pg") && row.running {
                    return Err(invalid());
                }
                if legacy.next() != Some(row) {
                    return Err(invalid());
                }
            }
        }
    }
    if legacy.next().is_some() {
        return Err(invalid());
    }
    // Hmount uses the separately adopted five-field projection and empty-string
    // Name for absent bind names, not the complete raw assets object above.
    #[derive(Serialize)]
    struct Row<'a> {
        destination: &'a str,
        source: &'a str,
        kind: &'a str,
        rw: bool,
        name: &'a str,
    }
    let bound = own.mounts.iter().try_fold(2_usize, |total, mount| {
        let strings = mount
            .destination
            .len()
            .checked_add(mount.source.len())
            .and_then(|n| n.checked_add(mount.kind.len()))
            .and_then(|n| n.checked_add(mount.name.as_ref().map_or(0, String::len)))
            .ok_or_else(invalid)?;
        // JSON escaping emits at most six bytes per original UTF-8 byte;
        // fixed keys, punctuation and one bool fit this fixed row overhead.
        total
            .checked_add(strings.checked_mul(6).ok_or_else(invalid)?)
            .and_then(|n| n.checked_add(256))
            .filter(|n| *n <= RECORD_CAP)
            .ok_or_else(invalid)
    })?;
    pay(
        meter,
        bound
            .checked_mul(64)
            .and_then(|n| {
                n.checked_add(
                    own.mounts
                        .len()
                        .checked_mul(std::mem::size_of::<Row<'_>>())?,
                )
            })
            .ok_or_else(invalid)?,
        own.mounts.len(),
    )?;
    let rows: Vec<Row<'_>> = own
        .mounts
        .iter()
        .map(|m| Row {
            destination: &m.destination,
            source: &m.source,
            kind: &m.kind,
            rw: m.rw,
            name: m.name.as_deref().unwrap_or(""),
        })
        .collect();
    let canonical = registry::canonical(&rows)?;
    if canonical.len() > bound || hash(&canonical, meter)? != installed.mount_digest {
        return Err(invalid());
    }
    Ok(())
}
fn same_before_after(
    before: &HostCutoverData,
    after: &HostCutoverData,
    installed: &InstalledMountData,
    meter: &SharedJournalBudget,
) -> Result<(), BackupError> {
    check_projection(before, installed, meter)?;
    check_projection(after, installed, meter)?;
    // Equality covers all-state container IDs/order, mounts, process/start,
    // restart/image/project/service and the full assets property's presence.
    pay(meter, RECORD_CAP.checked_mul(2).ok_or_else(invalid)?, 0)?;
    if before != after {
        return Err(invalid());
    }
    Ok(())
}

#[derive(PartialEq, Eq)]
struct Stamp {
    dev: u64,
    ino: u64,
    uid: u32,
    gid: u32,
    mode: u32,
    links: u64,
    bytes: u64,
    mtime: i64,
    mtime_ns: i64,
    ctime: i64,
    ctime_ns: i64,
}
impl Stamp {
    fn read(m: &Metadata) -> Self {
        Self {
            dev: m.dev(),
            ino: m.ino(),
            uid: m.uid(),
            gid: m.gid(),
            mode: m.mode(),
            links: m.nlink(),
            bytes: m.len(),
            mtime: m.mtime(),
            mtime_ns: m.mtime_nsec(),
            ctime: m.ctime(),
            ctime_ns: m.ctime_nsec(),
        }
    }
}
struct HeldBytesData {
    file: Arc<File>,
    stamp: Stamp,
    raw: Vec<u8>,
    digest: [u8; 32],
    scan: SharedJournalBudget,
}
impl HeldBytesData {
    fn open(
        parent: &BackupDir,
        leaf: &str,
        cap: usize,
        meter: &SharedJournalBudget,
    ) -> Result<Self, BackupError> {
        // open FD + secure_dir metadata + BackupDir nlink metadata + caller metadata.
        pay(meter, leaf.len().checked_add(1).ok_or_else(invalid)?, 4)?;
        let file = parent.open_file(leaf)?;
        let meta = file.metadata()?;
        if !meta.is_file()
            || meta.uid() != 0
            || meta.gid() != 0
            || meta.mode() & 0o7777 != 0o600
            || meta.nlink() != 1
            || meta.len() == 0
            || meta.len() > cap as u64
        {
            return Err(invalid());
        }
        let stamp = Stamp::read(&meta);
        let count = usize::try_from(meta.len()).map_err(|_| invalid())?;
        // Allocate/read payload and one EOF byte; reserve EOF/readback metadata
        // observations before either operation, including a failed readback.
        pay(meter, count.checked_add(1).ok_or_else(invalid)?, 2)?;
        let mut raw = vec![0; count];
        file_exact_prepaid(&file, &mut raw, meter)?;
        let mut extra = [0];
        if file_read_prepaid(&file, &mut extra, meter)? != 0
            || Stamp::read(&file.metadata()?) != stamp
        {
            return Err(invalid());
        }
        pay(meter, count, 0)?;
        let digest = Sha256::digest(&raw).into();
        let held = Self {
            file: Arc::new(file),
            stamp,
            raw,
            digest,
            scan: meter.clone(),
        };
        held.recheck(parent, leaf, meter)?;
        Ok(held)
    }
    fn recheck(
        &self,
        parent: &BackupDir,
        leaf: &str,
        meter: &SharedJournalBudget,
    ) -> Result<(), BackupError> {
        if !self.scan.shares_meter_with(meter) {
            return Err(invalid());
        }
        // Both full read/hash passes + EOF bytes. The fixed 16 KiB covers
        // read buffer, hash state, name CString and temporary FD/metadata state.
        // 16 entries bound clone/open, two internal named metadata checks,
        // four held/named before/after metadata checks, seeks and EOF probes.
        pay(
            meter,
            self.raw
                .len()
                .checked_mul(4)
                .and_then(|n| n.checked_add(16 * 1024 + 2))
                .ok_or_else(invalid)?,
            16,
        )?;
        let named = parent.open_file(leaf)?;
        for file in [self.file.as_ref(), &named] {
            if Stamp::read(&file.metadata()?) != self.stamp {
                return Err(invalid());
            }
            let mut position = file;
            position.seek(SeekFrom::Start(0))?;
            let mut offset = 0;
            let mut buffer = [0; 8192];
            let mut digest = Sha256::new();
            while offset < self.raw.len() {
                let count = buffer.len().min(self.raw.len() - offset);
                file_exact_prepaid(file, &mut buffer[..count], meter)?;
                if buffer[..count] != self.raw[offset..offset + count] {
                    return Err(invalid());
                }
                digest.update(&buffer[..count]);
                offset += count;
            }
            let mut extra = [0];
            let actual_digest: [u8; 32] = digest.finalize().into();
            if file_read_prepaid(file, &mut extra, meter)? != 0
                || actual_digest != self.digest
                || Stamp::read(&file.metadata()?) != self.stamp
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

// Do not hide EINTR retries in Read::read_exact/Write::write_all. Each actual
// native read/write attempt prepays its requested bytes and entry, including
// short IO, EOF, errors and interruptions. No retry can reset the scan budget.
fn file_read_prepaid(
    file: &File,
    bytes: &mut [u8],
    scan: &SharedJournalBudget,
) -> Result<usize, BackupError> {
    loop {
        pay(scan, bytes.len(), 1)?;
        let count = unsafe { libc::read(file.as_raw_fd(), bytes.as_mut_ptr().cast(), bytes.len()) };
        if count < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error.into());
        }
        return usize::try_from(count).map_err(|_| invalid());
    }
}
fn file_exact_prepaid(
    file: &File,
    mut bytes: &mut [u8],
    scan: &SharedJournalBudget,
) -> Result<(), BackupError> {
    while !bytes.is_empty() {
        let count = file_read_prepaid(file, bytes, scan)?;
        if count == 0 {
            return Err(invalid());
        }
        bytes = bytes.get_mut(count..).ok_or_else(invalid)?;
    }
    Ok(())
}
fn file_write_all_prepaid(
    file: &File,
    mut bytes: &[u8],
    deadline: Instant,
    stop: &AtomicBool,
    scan: &SharedJournalBudget,
) -> Result<(), BackupError> {
    while !bytes.is_empty() {
        root_io_boundary(deadline, stop)?;
        pay(scan, bytes.len(), 1)?;
        let count = unsafe { libc::write(file.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) };
        root_io_boundary(deadline, stop)?;
        if count < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error.into());
        }
        if count == 0 {
            return Err(invalid());
        }
        bytes = bytes.get(count as usize..).ok_or_else(invalid)?;
    }
    Ok(())
}

// Actual opened companion lineage and genesis bytes only. No RootJournalBytes,
// writable descriptor, append/replay/adoption method or authority conversion.
struct HeldGenesisData {
    companion: Arc<BackupDir>,
    journals_id: (u64, u64),
    binding: BackupDir,
    epochs_id: (u64, u64),
    epoch: BackupDir,
    binding_id: Uuid,
    epoch_id: Uuid,
    header: HeldBytesData,
    frame: HeldBytesData,
    broker_lock: File,
    broker_lock_stamp: Stamp,
    descriptor: crate::source::snapshot_effects::wire::Descriptor,
    descriptor_sha256: String,
    epoch_header_sha256: String,
    genesis_frame_sha256: String,
}
impl HeldGenesisData {
    // Only the eventual same-module OriginalRootJob may call this with its
    // before-held fixed companion handle and original authenticated notice.
    // The arguments are not exposed as a public path or qualification factory.
    fn read(
        companion: &Arc<BackupDir>,
        notice: &BrokerInstalledData,
        meter: &SharedJournalBudget,
    ) -> Result<Self, BackupError> {
        notice.validate()?;
        // Four fixed child opens + five private-directory metadata checks;
        // path/UUID Strings and the two CString copies per open fit 512 bytes.
        pay(meter, 512, 9)?;
        companion.require_private_directory()?;
        let journals = companion.open_dir("journals")?;
        let binding = journals.open_dir(&notice.binding_id.to_string())?;
        let journals_id = journals.identity()?;
        journals.require_private_directory()?;
        drop(journals);
        let epochs = binding.open_dir("epochs")?;
        let epoch = epochs.open_dir(&notice.epoch_id.to_string())?;
        let epochs_id = epochs.identity()?;
        epochs.require_private_directory()?;
        drop(epochs);
        for directory in [&binding, &epoch] {
            directory.require_private_directory()?;
        }
        // Only actual genesis exists before this broker is returned for work.
        exact_genesis_roster(&epoch, meter)?;
        let header = HeldBytesData::open(&epoch, "epoch.header", 40, meter)?;
        let frame = HeldBytesData::open(&epoch, GENESIS_NAME, FRAME_CAP, meter)?;
        let (descriptor, descriptor_sha256, epoch_header_sha256, genesis_frame_sha256) =
            read_genesis_bytes(&header.raw, &frame.raw, notice, meter)?;
        // The two companion identity calls and binding identity are real
        // independent metadata observations, separate from the lineage opens.
        pay(meter, 0, 3)?;
        if descriptor.companion_root_dev != companion.identity()?.0
            || descriptor.companion_root_ino != companion.identity()?.1
            || (descriptor.journal_root_dev, descriptor.journal_root_ino) != binding.identity()?
        {
            return Err(invalid());
        }
        // One FD open + both nested metadata checks + this caller's metadata.
        pay(meter, 64, 4)?;
        let broker_lock = binding.open_file("broker.lock")?;
        let metadata = broker_lock.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != 0
            || metadata.gid() != 0
            || metadata.mode() & 0o7777 != 0o600
            || metadata.nlink() != 1
            || metadata.len() != 0
        {
            return Err(invalid());
        }
        pay(meter, 0, 1)?; // clone the actual companion FD; no owner replacement.
        let held = Self {
            companion: Arc::clone(companion),
            journals_id,
            binding,
            epochs_id,
            epoch,
            binding_id: notice.binding_id,
            epoch_id: notice.epoch_id,
            header,
            frame,
            broker_lock,
            broker_lock_stamp: Stamp::read(&metadata),
            descriptor,
            descriptor_sha256,
            epoch_header_sha256,
            genesis_frame_sha256,
        };
        held.recheck(meter)?;
        Ok(held)
    }
    fn recheck(&self, meter: &SharedJournalBudget) -> Result<(), BackupError> {
        // Per link: child private metadata + named open + named identity +
        // held identity = four observations, four links. Roster/bytes/lock
        // have independent reservations below and cannot spend these twice.
        pay(meter, 512, 16)?;
        // The before companion, binding and epoch stay held. Intermediate
        // lineage directories carry immutable identities and are checked one
        // at a time against those originals; they never mint a new owner.
        let journals = self.companion.open_dir("journals")?;
        journals.require_private_directory()?;
        if journals.identity()? != self.journals_id
            || journals
                .open_dir(&self.binding_id.to_string())?
                .identity()?
                != self.binding.identity()?
        {
            return Err(invalid());
        }
        drop(journals);
        let epochs = self.binding.open_dir("epochs")?;
        epochs.require_private_directory()?;
        if epochs.identity()? != self.epochs_id
            || epochs.open_dir(&self.epoch_id.to_string())?.identity()? != self.epoch.identity()?
        {
            return Err(invalid());
        }
        drop(epochs);
        self.binding.require_private_directory()?;
        self.epoch.require_private_directory()?;
        exact_genesis_roster(&self.epoch, meter)?;
        self.header.recheck(&self.epoch, "epoch.header", meter)?;
        self.frame.recheck(&self.epoch, GENESIS_NAME, meter)?;
        // Held metadata + named FD open + its two nested metadata checks +
        // caller metadata, then the flock probe (and unlock on rejection).
        pay(meter, 64, 7)?;
        if Stamp::read(&self.broker_lock.metadata()?) != self.broker_lock_stamp
            || Stamp::read(&self.binding.open_file("broker.lock")?.metadata()?)
                != self.broker_lock_stamp
        {
            return Err(invalid());
        }
        if unsafe { libc::flock(self.broker_lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0
        {
            unsafe { libc::flock(self.broker_lock.as_raw_fd(), libc::LOCK_UN) };
            return Err(invalid());
        }
        if std::io::Error::last_os_error().raw_os_error() != Some(libc::EWOULDBLOCK) {
            return Err(invalid());
        }
        Ok(())
    }
}
fn exact_genesis_roster(epoch: &BackupDir, meter: &SharedJournalBudget) -> Result<(), BackupError> {
    // Each independent capped iterator may read cap+1 arbitrary NAME_MAX=255
    // names before rejecting, plus two dots/EOF and iterator FD bookkeeping.
    // Prepay before open/readdir/allocation, including failures; never refund.
    const CAP: usize = 2;
    const OBSERVED: usize = CAP + 1;
    let vector_slots = 4 * std::mem::size_of::<String>() * 3;
    let bytes = 64 * 1024 + OBSERVED * 255 * 2 + vector_slots;
    pay(meter, bytes, OBSERVED + 7)?;
    let mut names = epoch.list_bounded(CAP)?;
    names.sort_unstable();
    if names != [GENESIS_NAME.to_owned(), "epoch.header".to_owned()] {
        return Err(invalid());
    }
    Ok(())
}
fn read_genesis_bytes(
    header: &[u8],
    frame: &[u8],
    notice: &BrokerInstalledData,
    meter: &SharedJournalBudget,
) -> Result<
    (
        crate::source::snapshot_effects::wire::Descriptor,
        String,
        String,
        String,
    ),
    BackupError,
> {
    notice.validate()?;
    if header.len() != 40
        || &header[..8] != b"KWSJEPO2"
        || &header[8..24] != notice.epoch_id.as_bytes()
        || &header[24..40] != notice.binding_id.as_bytes()
        || !(120..=FRAME_CAP).contains(&frame.len())
        || &frame[..8] != b"KWSJBYT2"
        || frame[8..16] != 1_u64.to_be_bytes()
        || &frame[16..32] != notice.epoch_id.as_bytes()
        || &frame[32..48] != notice.binding_id.as_bytes()
        || u64::from_be_bytes(frame[112..120].try_into().map_err(|_| invalid())?)
            != (frame.len() - 120) as u64
    {
        return Err(invalid());
    }
    let epoch_header_sha256 = hash(header, meter)?;
    let descriptor_sha256 = hash(&frame[120..], meter)?;
    if hex::encode(&frame[48..80]) != epoch_header_sha256
        || hex::encode(&frame[80..112]) != descriptor_sha256
        || descriptor_sha256 != notice.descriptor_sha256
    {
        return Err(invalid());
    }
    // Descriptor payload is the original typed encoding, not canonical map
    // encoding. Parse it exactly once, reject duplicates/unknown fields, and
    // retain/hash its original bytes; never re-encode it to define its digest.
    pay(
        meter,
        (frame.len() - 120).checked_mul(64).ok_or_else(invalid)?,
        64,
    )?;
    let descriptor: crate::source::snapshot_effects::wire::Descriptor =
        serde_json::from_slice(&frame[120..])?;
    if descriptor.format_version != 2
        || descriptor.capability != "snapshot_effect_epoch_v2"
        || descriptor.storage_encoding != "root_frame_v1"
        || descriptor.binding_id != notice.binding_id
        || descriptor.epoch_id != notice.epoch_id
        || !v4(descriptor.deployment_id)
        || !v4(descriptor.source_group_id)
        || descriptor.broker_uid != 0
        || descriptor.broker_gid != 0
        || descriptor.broker_pid == 0
    {
        return Err(invalid());
    }
    let genesis_frame_sha256 = hash(frame, meter)?;
    Ok((
        descriptor,
        descriptor_sha256,
        epoch_header_sha256,
        genesis_frame_sha256,
    ))
}

// Exact wire15 mirror of learning-db snapshot_broker/b1_peer/job.rs.
// These decoded values grant no channel, process, broker, session or Root authority.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum RootControlData {
    SourceBootstrap {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        source_start_sha256: String,
    },
    SpawnCapture {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
    },
    CaptureBootstrap {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        source_start_sha256: String,
    },
    CaptureInit {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        backup_id: String,
        capture_nonce: String,
        request_id: String,
    },
    CaptureStartBound {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        backup_id: String,
        capture_start_sha256: String,
    },
    RequestInstallProof {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        backup_id: String,
        request_id: String,
    },
    InstallProofBound {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        backup_id: String,
        request_id: String,
        generation_binding_sha256: String,
    },
    BrokerInstalled {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        backup_id: String,
        request_id: String,
        binding_id: String,
        epoch_id: String,
        descriptor_sha256: String,
    },
    BrokerInstallObserved {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        backup_id: String,
        request_id: String,
        binding_id: String,
        epoch_id: String,
        descriptor_sha256: String,
        observation_sha256: String,
    },
    RequestCaptureProof {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        backup_id: String,
        request_id: String,
    },
    CaptureProofBound {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        backup_id: String,
        request_id: String,
        generation_binding_sha256: String,
    },
    CapturedDataReady {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        backup_id: String,
        captured_data_sha256: String,
    },
    SettleCapture {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        backup_id: String,
        captured_data_sha256: String,
    },
    CaptureSettled {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        backup_id: String,
        capture_settlement_sha256: String,
    },
    Abort {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        reason_code: String,
    },
}
impl RootControlData {
    fn kind(&self) -> &'static str {
        match self {
            Self::SourceBootstrap { .. } => "SourceBootstrap",
            Self::SpawnCapture { .. } => "SpawnCapture",
            Self::CaptureBootstrap { .. } => "CaptureBootstrap",
            Self::CaptureInit { .. } => "CaptureInit",
            Self::CaptureStartBound { .. } => "CaptureStartBound",
            Self::RequestInstallProof { .. } => "RequestInstallProof",
            Self::InstallProofBound { .. } => "InstallProofBound",
            Self::BrokerInstalled { .. } => "BrokerInstalled",
            Self::BrokerInstallObserved { .. } => "BrokerInstallObserved",
            Self::RequestCaptureProof { .. } => "RequestCaptureProof",
            Self::CaptureProofBound { .. } => "CaptureProofBound",
            Self::CapturedDataReady { .. } => "CapturedDataReady",
            Self::SettleCapture { .. } => "SettleCapture",
            Self::CaptureSettled { .. } => "CaptureSettled",
            Self::Abort { .. } => "Abort",
        }
    }
    fn common(&self) -> (&str, &str, u64) {
        match self {
            Self::SourceBootstrap {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::SpawnCapture {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::CaptureBootstrap {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::CaptureInit {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::CaptureStartBound {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::RequestInstallProof {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::InstallProofBound {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::BrokerInstalled {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::BrokerInstallObserved {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::RequestCaptureProof {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::CaptureProofBound {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::CapturedDataReady {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::SettleCapture {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::CaptureSettled {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::Abort {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
        }
    }
}

impl RootControlData {
    fn validate(&self) -> Result<(), BackupError> {
        let version = match self {
            Self::SourceBootstrap { schema_version, .. }
            | Self::SpawnCapture { schema_version, .. }
            | Self::CaptureBootstrap { schema_version, .. }
            | Self::CaptureInit { schema_version, .. }
            | Self::CaptureStartBound { schema_version, .. }
            | Self::RequestInstallProof { schema_version, .. }
            | Self::InstallProofBound { schema_version, .. }
            | Self::BrokerInstalled { schema_version, .. }
            | Self::BrokerInstallObserved { schema_version, .. }
            | Self::RequestCaptureProof { schema_version, .. }
            | Self::CaptureProofBound { schema_version, .. }
            | Self::CapturedDataReady { schema_version, .. }
            | Self::SettleCapture { schema_version, .. }
            | Self::CaptureSettled { schema_version, .. }
            | Self::Abort { schema_version, .. } => *schema_version,
        };
        let (job, cookie, _) = self.common();
        let correlated =
            |backup: &str, request: &str| registry::v4(backup) && registry::v4(request);
        let fields = match self {
            Self::SourceBootstrap {
                source_start_sha256,
                ..
            }
            | Self::CaptureBootstrap {
                source_start_sha256,
                ..
            } => crate::valid_digest(source_start_sha256),
            Self::SpawnCapture { .. } => true,
            Self::CaptureInit {
                backup_id,
                request_id,
                capture_nonce,
                ..
            } => correlated(backup_id, request_id) && crate::valid_digest(capture_nonce),
            Self::CaptureStartBound {
                backup_id,
                capture_start_sha256,
                ..
            } => registry::v4(backup_id) && crate::valid_digest(capture_start_sha256),
            Self::RequestInstallProof {
                backup_id,
                request_id,
                ..
            }
            | Self::RequestCaptureProof {
                backup_id,
                request_id,
                ..
            } => correlated(backup_id, request_id),
            Self::InstallProofBound {
                backup_id,
                request_id,
                generation_binding_sha256,
                ..
            }
            | Self::CaptureProofBound {
                backup_id,
                request_id,
                generation_binding_sha256,
                ..
            } => {
                correlated(backup_id, request_id) && crate::valid_digest(generation_binding_sha256)
            }
            Self::BrokerInstalled {
                backup_id,
                request_id,
                binding_id,
                epoch_id,
                descriptor_sha256,
                ..
            }
            | Self::BrokerInstallObserved {
                backup_id,
                request_id,
                binding_id,
                epoch_id,
                descriptor_sha256,
                ..
            } => {
                correlated(backup_id, request_id)
                    && registry::v4(binding_id)
                    && registry::v4(epoch_id)
                    && crate::valid_digest(descriptor_sha256)
                    && match self {
                        Self::BrokerInstallObserved {
                            observation_sha256, ..
                        } => crate::valid_digest(observation_sha256),
                        _ => true,
                    }
            }
            Self::CapturedDataReady {
                backup_id,
                captured_data_sha256,
                ..
            }
            | Self::SettleCapture {
                backup_id,
                captured_data_sha256,
                ..
            } => registry::v4(backup_id) && crate::valid_digest(captured_data_sha256),
            Self::CaptureSettled {
                backup_id,
                capture_settlement_sha256,
                ..
            } => registry::v4(backup_id) && crate::valid_digest(capture_settlement_sha256),
            Self::Abort { reason_code, .. } => [
                "InputShape",
                "ProcessChanged",
                "RootOwnerLost",
                "CookieMismatch",
                "Phase",
                "StaleInitialProof",
                "Budget",
                "ChildExit",
                "UnexpectedEof",
                "ExtraBytes",
                "Deadline",
                "SettlementUnconfirmed",
            ]
            .contains(&reason_code.as_str()),
        };
        if version != 1 || !registry::v4(job) || !crate::valid_digest(cookie) || !fields {
            return Err(invalid());
        }
        Ok(())
    }
}

// DATA labels for the two existing channels. They are not process roles or
// authenticated endpoints. The eventual Root owner must bind its actual FDs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RootChannel {
    Source,
    Capture,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum RootDirection {
    Receive,
    Send,
}

// One instance is embedded in one ledger, never one instance per endpoint.
// Wire reservations count actual prefix/body bytes. Native attempts, retries,
// parsing and allocations additionally consume the original scan meter.
struct RootWireBudget {
    frames: u64,
    bytes: u64,
}
impl RootWireBudget {
    fn prefix(&mut self, scan: &SharedJournalBudget) -> Result<(), BackupError> {
        let frames = self.frames.checked_add(1).ok_or_else(invalid)?;
        let bytes = self.bytes.checked_add(4).ok_or_else(invalid)?;
        if frames > CONTROL_FRAMES || bytes > CONTROL_BYTES {
            return Err(invalid());
        }
        pay(scan, 4, 1)?;
        self.frames = frames;
        self.bytes = bytes;
        Ok(())
    }
    fn body(&mut self, length: usize, scan: &SharedJournalBudget) -> Result<(), BackupError> {
        if length == 0 || length > CONTROL_CAP {
            return Err(invalid());
        }
        let bytes = self.bytes.checked_add(length as u64).ok_or_else(invalid)?;
        if bytes > CONTROL_BYTES {
            return Err(invalid());
        }
        pay(scan, length, 0)?;
        self.bytes = bytes;
        Ok(())
    }
    fn extra_probe(&mut self, scan: &SharedJournalBudget) -> Result<(), BackupError> {
        let bytes = self.bytes.checked_add(1).ok_or_else(invalid)?;
        if bytes > CONTROL_BYTES {
            return Err(invalid());
        }
        pay(scan, 1, 1)?;
        self.bytes = bytes;
        Ok(())
    }
}

// No Clone, Default, public constructor or conversion to OriginalRootJob. A
// ledger recognizes DATA and performs bounded IO; it cannot qualify a process.
// Both endpoints must borrow this same object, including its sticky failure,
// phase, four sequence counters and the one non-refundable wire/scan budget.
struct RootControlLedger {
    scan: SharedJournalBudget,
    wire: RootWireBudget,
    failed: bool,
    step: usize,
    next: [[u64; 2]; 2],
    job: String,
    cookie: String,
    source_start: Option<String>,
    capture_start: Option<String>,
    backup: Option<String>,
    requests: [Option<String>; 3],
    install_generation: Option<String>,
    installed: Option<BrokerInstalledData>,
    captured: Option<String>,
    total: Instant,
    pending: Option<Instant>,
    cutover: Option<Instant>,
}
impl RootControlLedger {
    fn data_only(
        scan: SharedJournalBudget,
        job: &str,
        cookie: &str,
        total: Instant,
    ) -> Result<Self, BackupError> {
        pay(&scan, 4096, 1)?;
        if !registry::v4(job) || !crate::valid_digest(cookie) || Instant::now() >= total {
            return Err(invalid());
        }
        Ok(Self {
            scan,
            wire: RootWireBudget {
                frames: 0,
                bytes: 0,
            },
            failed: false,
            step: 0,
            next: [[0; 2]; 2],
            job: job.to_owned(),
            cookie: cookie.to_owned(),
            source_start: None,
            capture_start: None,
            backup: None,
            requests: [None, None, None],
            install_generation: None,
            installed: None,
            captured: None,
            total,
            pending: None,
            cutover: None,
        })
    }
    fn begin(&mut self) -> Result<(), BackupError> {
        if self.failed {
            return Err(invalid());
        }
        // Any error, unwind or incomplete IO leaves the original ledger dead.
        self.failed = true;
        Ok(())
    }
    fn deadline(&self) -> Instant {
        self.pending
            .unwrap_or(self.total)
            .min(self.cutover.unwrap_or(self.total))
            .min(self.total)
    }
    fn boundary(&self, stop: &AtomicBool) -> Result<(), BackupError> {
        if stop.load(Ordering::Acquire) || Instant::now() >= self.deadline() {
            return Err(invalid());
        }
        Ok(())
    }
    fn observe(
        &mut self,
        data: &RootControlData,
        channel: RootChannel,
        direction: RootDirection,
        now: Instant,
    ) -> Result<(), BackupError> {
        data.validate()?;
        if now >= self.deadline() {
            return Err(invalid());
        }
        let (job, cookie, sequence) = data.common();
        let c = usize::from(channel == RootChannel::Capture);
        let d = usize::from(direction == RootDirection::Send);
        if job != self.job || cookie != self.cookie || sequence != self.next[c][d] {
            return Err(invalid());
        }
        use RootChannel::{Capture as C, Source as S};
        use RootDirection::{Receive as R, Send as W};
        const ORDER: &[(RootChannel, RootDirection, &str)] = &[
            (S, W, "SourceBootstrap"),
            (S, R, "SpawnCapture"),
            (C, W, "CaptureBootstrap"),
            (C, R, "CaptureInit"),
            (C, W, "CaptureStartBound"),
            (C, R, "RequestInstallProof"),
            (C, W, "InstallProofBound"),
            (C, R, "BrokerInstalled"),
            (C, W, "BrokerInstallObserved"),
            (C, R, "RequestCaptureProof"),
            (C, W, "CaptureProofBound"),
            (C, R, "CapturedDataReady"),
            (S, W, "CapturedDataReady"),
            (S, R, "SettleCapture"),
            (C, W, "SettleCapture"),
            (S, W, "CaptureSettled"),
        ];
        if ORDER.get(self.step) != Some(&(channel, direction, data.kind())) {
            return Err(invalid());
        }
        // All retained string clones are bounded by the validated wire15
        // schema and prepaid before the first clone, even on later failure.
        pay(&self.scan, CONTROL_CAP, 16)?;
        match data {
            RootControlData::SourceBootstrap {
                source_start_sha256,
                ..
            } => self.source_start = Some(source_start_sha256.clone()),
            RootControlData::CaptureBootstrap {
                source_start_sha256,
                ..
            } => {
                if self.source_start.as_deref() != Some(source_start_sha256) {
                    return Err(invalid());
                }
            }
            RootControlData::CaptureInit {
                backup_id,
                request_id,
                ..
            } => {
                self.backup = Some(backup_id.clone());
                self.requests[0] = Some(request_id.clone());
            }
            RootControlData::CaptureStartBound {
                backup_id,
                capture_start_sha256,
                ..
            } => {
                if self.backup.as_deref() != Some(backup_id) {
                    return Err(invalid());
                }
                self.capture_start = Some(capture_start_sha256.clone());
            }
            RootControlData::RequestInstallProof {
                backup_id,
                request_id,
                ..
            }
            | RootControlData::RequestCaptureProof {
                backup_id,
                request_id,
                ..
            } => {
                if self.backup.as_deref() != Some(backup_id)
                    || self.requests.iter().flatten().any(|old| old == request_id)
                {
                    return Err(invalid());
                }
                let slot = if matches!(data, RootControlData::RequestInstallProof { .. }) {
                    1
                } else {
                    2
                };
                if self.requests[slot].is_some() {
                    return Err(invalid());
                }
                self.requests[slot] = Some(request_id.clone());
            }
            RootControlData::InstallProofBound {
                backup_id,
                request_id,
                generation_binding_sha256,
                ..
            }
            | RootControlData::CaptureProofBound {
                backup_id,
                request_id,
                generation_binding_sha256,
                ..
            } => {
                let install = matches!(data, RootControlData::InstallProofBound { .. });
                if self.backup.as_deref() != Some(backup_id)
                    || self.requests[if install { 1 } else { 2 }].as_deref() != Some(request_id)
                {
                    return Err(invalid());
                }
                if install {
                    self.install_generation = Some(generation_binding_sha256.clone());
                }
            }
            RootControlData::BrokerInstalled {
                backup_id,
                request_id,
                binding_id,
                epoch_id,
                descriptor_sha256,
                ..
            } => {
                if self.backup.as_deref() != Some(backup_id)
                    || self.requests[1].as_deref() != Some(request_id)
                    || self.installed.is_some()
                {
                    return Err(invalid());
                }
                self.installed = Some(BrokerInstalledData {
                    schema_version: 1,
                    kind: "BrokerInstalled".into(),
                    job_id: Uuid::parse_str(job).map_err(|_| invalid())?,
                    root_cookie: cookie.to_owned(),
                    sequence,
                    backup_id: Uuid::parse_str(backup_id).map_err(|_| invalid())?,
                    request_id: Uuid::parse_str(request_id).map_err(|_| invalid())?,
                    binding_id: Uuid::parse_str(binding_id).map_err(|_| invalid())?,
                    epoch_id: Uuid::parse_str(epoch_id).map_err(|_| invalid())?,
                    descriptor_sha256: descriptor_sha256.clone(),
                });
            }
            RootControlData::BrokerInstallObserved {
                backup_id,
                request_id,
                binding_id,
                epoch_id,
                descriptor_sha256,
                ..
            } => {
                let original = self.installed.as_ref().ok_or_else(invalid)?;
                if self.backup.as_deref() != Some(backup_id)
                    || self.requests[1].as_deref() != Some(request_id)
                    || original.binding_id.to_string() != *binding_id
                    || original.epoch_id.to_string() != *epoch_id
                    || original.descriptor_sha256 != *descriptor_sha256
                {
                    return Err(invalid());
                }
            }
            RootControlData::CapturedDataReady {
                backup_id,
                captured_data_sha256,
                ..
            } => {
                if self.backup.as_deref() != Some(backup_id)
                    || self
                        .captured
                        .as_deref()
                        .is_some_and(|v| v != captured_data_sha256)
                {
                    return Err(invalid());
                }
                self.captured = Some(captured_data_sha256.clone());
            }
            RootControlData::SettleCapture {
                backup_id,
                captured_data_sha256,
                ..
            } => {
                if self.backup.as_deref() != Some(backup_id)
                    || self.captured.as_deref() != Some(captured_data_sha256)
                {
                    return Err(invalid());
                }
            }
            RootControlData::CaptureSettled { backup_id, .. }
                if self.backup.as_deref() != Some(backup_id) =>
            {
                return Err(invalid());
            }
            _ => {}
        }
        let request_end = || now.checked_add(Duration::from_secs(30)).ok_or_else(invalid);
        match self.step {
            // The first native byte may already have opened this request;
            // decoding/observing a frame must never refresh that clock.
            0 | 2 | 5 | 7 | 9 | 11 | 13 if self.pending.is_none() => {
                self.pending = Some(request_end()?.min(self.total))
            }
            1 | 4 | 6 | 8 | 10 | 12 | 15 => self.pending = None,
            _ => {}
        }
        if self.step == 5 && self.cutover.is_none() {
            self.cutover = Some(
                now.checked_add(Duration::from_secs(120))
                    .ok_or_else(invalid)?
                    .min(self.total),
            );
        } else if self.step == 8 {
            self.cutover = None;
        }
        self.next[c][d] = sequence.checked_add(1).ok_or_else(invalid)?;
        self.step += 1;
        Ok(())
    }
    fn receive_data(
        &mut self,
        channel: RootChannel,
        socket: &UnixStream,
        stop: &AtomicBool,
    ) -> Result<RootControlData, BackupError> {
        self.begin()?;
        self.boundary(stop)?;
        self.wire.prefix(&self.scan)?;
        let mut prefix = [0; 4];
        root_socket_exact(socket, &mut prefix[..1], self.deadline(), stop, &self.scan)?;
        // Waiting for an operation uses its existing total/deadline. Once any
        // byte arrives, one request clock covers remaining prefix, body, parse,
        // record work and reply; a slow prefix cannot postpone that clock.
        let first = Instant::now();
        if self.pending.is_none() {
            self.pending = Some(
                first
                    .checked_add(Duration::from_secs(30))
                    .ok_or_else(invalid)?
                    .min(self.total),
            );
        }
        if self.step == 5 && self.cutover.is_none() {
            self.cutover = Some(
                first
                    .checked_add(Duration::from_secs(120))
                    .ok_or_else(invalid)?
                    .min(self.total),
            );
        }
        root_socket_exact(socket, &mut prefix[1..], self.deadline(), stop, &self.scan)?;
        let length = u32::from_be_bytes(prefix) as usize;
        self.wire.body(length, &self.scan)?;
        let deadline = self.deadline();
        // Wire reservation also prepays this exact body allocation.
        let mut body = vec![0; length];
        root_socket_exact(socket, &mut body, self.deadline(), stop, &self.scan)?;
        let data = parse::<RootControlData>(&body, CONTROL_CAP, &self.scan)?;
        self.observe(&data, channel, RootDirection::Receive, Instant::now())?;
        root_io_boundary(deadline, stop)?;
        self.boundary(stop)?;
        self.failed = false;
        Ok(data)
    }
    fn send_data(
        &mut self,
        channel: RootChannel,
        socket: &UnixStream,
        data: &RootControlData,
        stop: &AtomicBool,
    ) -> Result<(), BackupError> {
        self.begin()?;
        // Only the durable retained publication below can send an ACK. DATA
        // shape/phase recognition cannot bypass its file/parent/readback gate.
        if matches!(data, RootControlData::BrokerInstallObserved { .. }) {
            return Err(invalid());
        }
        self.send_inner(channel, socket, data, stop)?;
        self.failed = false;
        Ok(())
    }
    fn send_inner(
        &mut self,
        channel: RootChannel,
        socket: &UnixStream,
        data: &RootControlData,
        stop: &AtomicBool,
    ) -> Result<(), BackupError> {
        self.boundary(stop)?;
        let deadline = self.deadline();
        data.validate()?;
        pay(&self.scan, CONTROL_CAP * 64, 64)?;
        let body = registry::canonical(data)?;
        self.wire.prefix(&self.scan)?;
        self.wire.body(body.len(), &self.scan)?;
        self.observe(data, channel, RootDirection::Send, Instant::now())?;
        root_socket_all(
            socket,
            &(body.len() as u32).to_be_bytes(),
            deadline.min(self.deadline()),
            stop,
            &self.scan,
        )?;
        root_socket_all(
            socket,
            &body,
            deadline.min(self.deadline()),
            stop,
            &self.scan,
        )?;
        self.boundary(stop)
    }
    fn require_idle_no_extra(
        &mut self,
        socket: &UnixStream,
        stop: &AtomicBool,
    ) -> Result<(), BackupError> {
        self.begin()?;
        loop {
            self.boundary(stop)?;
            self.wire.extra_probe(&self.scan)?;
            let mut extra = 0_u8;
            let count = unsafe {
                libc::recv(
                    socket.as_raw_fd(),
                    (&mut extra as *mut u8).cast(),
                    1,
                    libc::MSG_PEEK | libc::MSG_DONTWAIT,
                )
            };
            self.boundary(stop)?;
            if count < 0 {
                match std::io::Error::last_os_error().kind() {
                    std::io::ErrorKind::Interrupted => continue,
                    std::io::ErrorKind::WouldBlock => {
                        self.failed = false;
                        return Ok(());
                    }
                    _ => return Err(invalid()),
                }
            }
            // Both early EOF and a queued extra byte are permanent negatives.
            return Err(invalid());
        }
    }
}

fn root_io_boundary(deadline: Instant, stop: &AtomicBool) -> Result<(), BackupError> {
    if (stop.load(Ordering::Acquire) && !root_runtime::cleanup_io()) || Instant::now() >= deadline {
        return Err(invalid());
    }
    Ok(())
}
fn root_socket_poll(
    socket: &UnixStream,
    events: i16,
    deadline: Instant,
    stop: &AtomicBool,
    scan: &SharedJournalBudget,
) -> Result<(), BackupError> {
    loop {
        root_io_boundary(deadline, stop)?;
        pay(scan, 0, 1)?;
        let milliseconds = deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(250)
            .saturating_add(1) as i32;
        let mut item = libc::pollfd {
            fd: socket.as_raw_fd(),
            events,
            revents: 0,
        };
        let count = unsafe { libc::poll(&mut item, 1, milliseconds) };
        root_io_boundary(deadline, stop)?;
        if count < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        if count < 0 || item.revents & libc::POLLNVAL != 0 {
            return Err(invalid());
        }
        if count == 0 {
            continue;
        }
        // HUP with readable bytes must still drain the actual short tail;
        // recv(0) decides EOF. No take(cap) or synthetic EOF is used.
        if item.revents & (events | libc::POLLHUP) != 0 {
            return Ok(());
        }
        return Err(invalid());
    }
}
fn root_socket_exact(
    socket: &UnixStream,
    mut remaining: &mut [u8],
    deadline: Instant,
    stop: &AtomicBool,
    scan: &SharedJournalBudget,
) -> Result<(), BackupError> {
    while !remaining.is_empty() {
        root_socket_poll(socket, libc::POLLIN, deadline, stop, scan)?;
        pay(scan, remaining.len(), 1)?;
        let count = unsafe {
            libc::recv(
                socket.as_raw_fd(),
                remaining.as_mut_ptr().cast(),
                remaining.len(),
                libc::MSG_DONTWAIT,
            )
        };
        root_io_boundary(deadline, stop)?;
        if count <= 0 {
            if count < 0
                && matches!(
                    std::io::Error::last_os_error().kind(),
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                )
            {
                continue;
            }
            return Err(invalid());
        }
        remaining = remaining.get_mut(count as usize..).ok_or_else(invalid)?;
    }
    root_io_boundary(deadline, stop)
}
fn root_socket_all(
    socket: &UnixStream,
    mut remaining: &[u8],
    deadline: Instant,
    stop: &AtomicBool,
    scan: &SharedJournalBudget,
) -> Result<(), BackupError> {
    while !remaining.is_empty() {
        root_socket_poll(socket, libc::POLLOUT, deadline, stop, scan)?;
        pay(scan, remaining.len(), 1)?;
        let count = unsafe {
            libc::send(
                socket.as_raw_fd(),
                remaining.as_ptr().cast(),
                remaining.len(),
                libc::MSG_NOSIGNAL | libc::MSG_DONTWAIT,
            )
        };
        root_io_boundary(deadline, stop)?;
        if count <= 0 {
            if count < 0
                && matches!(
                    std::io::Error::last_os_error().kind(),
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                )
            {
                continue;
            }
            return Err(invalid());
        }
        remaining = remaining.get(count as usize..).ok_or_else(invalid)?;
    }
    root_io_boundary(deadline, stop)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstallGenerationData {
    schema_version: u64,
    capability: String,
    job_id: String,
    backup_id: String,
    phase: String,
    request_id: String,
    capture_start_sha256: String,
    isolation_bytes: u64,
    isolation_sha256: String,
    inspection_bytes: u64,
    inspection_sha256: String,
    lock_dev: u64,
    lock_ino: u64,
    observed_unix_ms: u64,
}
impl InstallGenerationData {
    fn join(&self, ledger: &RootControlLedger) -> Result<(), BackupError> {
        if self.schema_version != 1
            || self.capability != "b1_immutable_isolation_generation_v1"
            || self.phase != "Install"
            || self.job_id != ledger.job
            || ledger.backup.as_deref() != Some(self.backup_id.as_str())
            || ledger.requests[1].as_deref() != Some(self.request_id.as_str())
            || ledger.capture_start.as_deref() != Some(self.capture_start_sha256.as_str())
            || !registry::v4(&self.job_id)
            || !registry::v4(&self.backup_id)
            || !registry::v4(&self.request_id)
            || !(1..=4096).contains(&self.isolation_bytes)
            || !(1..=RECORD_CAP as u64).contains(&self.inspection_bytes)
            || self.lock_dev == 0
            || self.lock_ino == 0
            || self.observed_unix_ms == 0
            || [
                &self.capture_start_sha256,
                &self.isolation_sha256,
                &self.inspection_sha256,
            ]
            .into_iter()
            .any(|v| !crate::valid_digest(v))
        {
            return Err(invalid());
        }
        Ok(())
    }
}

// Borrow original held objects; never reopen a caller-selected path/latest
// generation, replace the before companion, or reacquire a generation lock.
// The future Root owner must have acquired and continuously retained the real
// job/Install locks and actual collector objects before using this DATA slice.
struct InstallPublicationInputs<'a> {
    backup_parent: &'a BackupDir,
    install: &'a BackupDir,
    generation: &'a HeldBytesData,
    proof: &'a HeldBytesData,
    inspection: &'a HeldBytesData,
    generation_lock: &'a File,
    before_companion: &'a BackupDir,
    before: &'a HostCutoverData,
    installed: &'a InstalledMountData,
    genesis: &'a HeldGenesisData,
}

// Retains the exact create FD even after any write/sync/readback failure. The
// create_file primitive is O_WRONLY|O_CREAT|O_EXCL|O_NOFOLLOW|O_CLOEXEC,0600;
// therefore the original write FD is checked by full stamp while a retained
// read FD and independent named FD compare every original byte/hash. We never
// replace the original create FD with a reopened descriptor or chmod to0400.
struct InstallObservationPublication<'a> {
    input: InstallPublicationInputs<'a>,
    generation: InstallGenerationData,
    lock_stamp: Stamp,
    scan: SharedJournalBudget,
    attempted: bool,
    ack_attempted: bool,
    failed: bool,
    durable: bool,
    file: Option<File>,
    writer_stamp: Option<Stamp>,
    raw: Vec<u8>,
    data: Option<InstallCutoverObservationData>,
    readback: Option<HeldBytesData>,
}
impl<'a> InstallObservationPublication<'a> {
    fn hold_data(
        input: InstallPublicationInputs<'a>,
        ledger: &mut RootControlLedger,
        stop: &AtomicBool,
    ) -> Result<Self, BackupError> {
        ledger.begin()?;
        ledger.boundary(stop)?;
        if ledger.step != 8 {
            return Err(invalid());
        }
        let generation: InstallGenerationData =
            parse(&input.generation.raw, CONTROL_CAP, &ledger.scan)?;
        generation.join(ledger)?;
        pay(&ledger.scan, 0, 1)?;
        let lock_stamp = Stamp::read(&input.generation_lock.metadata()?);
        let value = Self {
            input,
            generation,
            lock_stamp,
            scan: ledger.scan.clone(),
            attempted: false,
            ack_attempted: false,
            failed: false,
            durable: false,
            file: None,
            writer_stamp: None,
            raw: Vec::new(),
            data: None,
            readback: None,
        };
        value.recheck_inputs(ledger, false, stop)?;
        ledger.failed = false;
        Ok(value)
    }
    fn recheck_inputs(
        &self,
        ledger: &RootControlLedger,
        observed: bool,
        stop: &AtomicBool,
    ) -> Result<(), BackupError> {
        if !self.scan.shares_meter_with(&ledger.scan) || ledger.step != 8 {
            return Err(invalid());
        }
        ledger.boundary(stop)?;
        self.generation.join(ledger)?;
        // Named install and the same before-held companion must still be the
        // original objects. The reference to HeldGenesisData retains its exact
        // original FDs and all three independently hashed byte domains.
        pay(&self.scan, 512, 8)?;
        self.input.install.require_private_directory()?;
        self.input.before_companion.require_private_directory()?;
        if self.input.backup_parent.open_dir("install")?.identity()?
            != self.input.install.identity()?
            || self.input.before_companion.identity()? != self.input.genesis.companion.identity()?
        {
            return Err(invalid());
        }
        let notice = ledger.installed.as_ref().ok_or_else(invalid)?;
        notice.validate()?;
        if self.input.genesis.binding_id != notice.binding_id
            || self.input.genesis.epoch_id != notice.epoch_id
            || self.input.genesis.descriptor_sha256 != notice.descriptor_sha256
        {
            return Err(invalid());
        }
        self.input.genesis.recheck(&self.scan)?;
        pay(&self.scan, 1024, 0)?;
        let backup = notice.backup_id.to_string();
        self.input
            .generation
            .recheck(self.input.install, "generation-binding.json", &self.scan)?;
        self.input.proof.recheck(
            self.input.install,
            &format!("isolation-{backup}.json"),
            &self.scan,
        )?;
        self.input.inspection.recheck(
            self.input.install,
            &format!("inspection-{backup}.json"),
            &self.scan,
        )?;
        if ledger.install_generation.as_deref()
            != Some(hex::encode(self.input.generation.digest).as_str())
            || self.input.proof.raw.len() as u64 != self.generation.isolation_bytes
            || hex::encode(self.input.proof.digest) != self.generation.isolation_sha256
            || self.input.inspection.raw.len() as u64 != self.generation.inspection_bytes
            || hex::encode(self.input.inspection.digest) != self.generation.inspection_sha256
        {
            return Err(invalid());
        }
        // This verifies continued identity/metadata of the supplied original
        // lock FD. It does not acquire a fresh lock or certify Root admission.
        pay(&self.scan, 128, 6)?;
        let held = self.input.generation_lock.metadata()?;
        let named = self
            .input
            .install
            .open_file(&format!("isolation-{backup}.lock"))?
            .metadata()?;
        if !held.is_file()
            || held.uid() != 0
            || held.gid() != 0
            || held.mode() & 0o7777 != 0o600
            || held.nlink() != 1
            || held.len() != 0
            || held.dev() != self.generation.lock_dev
            || held.ino() != self.generation.lock_ino
            || Stamp::read(&held) != self.lock_stamp
            || Stamp::read(&named) != self.lock_stamp
        {
            return Err(invalid());
        }
        install_roster(self.input.install, &backup, observed, &self.scan)?;
        ledger.boundary(stop)
    }
    fn publish_data(
        &mut self,
        after: HostCutoverData,
        ledger: &mut RootControlLedger,
        stop: &AtomicBool,
    ) -> Result<(), BackupError> {
        ledger.begin()?;
        if self.failed || self.attempted || self.durable {
            self.failed = true;
            return Err(invalid());
        }
        self.failed = true;
        self.attempted = true;
        self.recheck_inputs(ledger, false, stop)?;
        // These are complete observed DATA comparisons only. The absent Root
        // collector must supply its actual all-host before/after and separately
        // check network/native/code; a notice cannot manufacture those facts.
        same_before_after(self.input.before, &after, self.input.installed, &self.scan)?;
        pay(&self.scan, 4096, 1)?;
        let now = u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| invalid())?
                .as_millis(),
        )
        .map_err(|_| invalid())?;
        if now < self.generation.observed_unix_ms {
            return Err(invalid());
        }
        let notice = ledger.installed.as_ref().ok_or_else(invalid)?;
        let data = InstallCutoverObservationData {
            schema_version: 1,
            capability: "b1_install_cutover_observation_v1".into(),
            job_id: notice.job_id,
            backup_id: notice.backup_id,
            phase: "Install".into(),
            request_id: notice.request_id,
            binding_id: notice.binding_id,
            epoch_id: notice.epoch_id,
            generation_binding_sha256: ledger
                .install_generation
                .as_ref()
                .ok_or_else(invalid)?
                .clone(),
            descriptor_sha256: self.input.genesis.descriptor_sha256.clone(),
            epoch_header_sha256: self.input.genesis.epoch_header_sha256.clone(),
            genesis_frame_sha256: self.input.genesis.genesis_frame_sha256.clone(),
            observed_unix_ms: now,
            host_cutover: after,
        };
        data.matches_notice(notice)?;
        self.raw = encode_observation(&data, &self.scan)?;
        // Typed validation and correlation happen before creating a leaf.
        let checked = InstallCutoverObservationData::read_data(&self.raw, &self.scan)?;
        checked.matches_notice(notice)?;
        self.data = Some(data);
        ledger.boundary(stop)?;
        // Exact single attempt; an existing leaf or symlink is a permanent
        // negative. No overwrite, rename, unlink, repair or retry follows.
        pay(&self.scan, OBSERVATION_NAME.len() + 1, 2)?;
        self.file = Some(self.input.install.create_file(OBSERVATION_NAME)?);
        let file = self.file.as_ref().ok_or_else(invalid)?;
        file_write_all_prepaid(file, &self.raw, ledger.deadline(), stop, &self.scan)?;
        pay(&self.scan, 0, 1)?;
        let metadata = file.metadata()?;
        let stamp = Stamp::read(&metadata);
        if !metadata.is_file()
            || metadata.uid() != 0
            || metadata.gid() != 0
            || metadata.mode() & 0o7777 != 0o600
            || metadata.nlink() != 1
            || metadata.len() != self.raw.len() as u64
        {
            self.writer_stamp = Some(stamp);
            return Err(invalid());
        }
        self.writer_stamp = Some(stamp);
        ledger.boundary(stop)?;
        pay(&self.scan, 0, 1)?;
        file.sync_all()?;
        ledger.boundary(stop)?;
        pay(&self.scan, 0, 1)?;
        self.input.install.sync()?;
        ledger.boundary(stop)?;
        self.readback = Some(HeldBytesData::open(
            self.input.install,
            OBSERVATION_NAME,
            RECORD_CAP,
            &self.scan,
        )?);
        self.recheck_published(ledger, stop)?;
        // This is a durable DATA publication state, never Root/process/cut
        // qualification. Only reaching this point permits ACK encoding.
        self.durable = true;
        self.failed = false;
        ledger.failed = false;
        Ok(())
    }
    fn recheck_published(
        &self,
        ledger: &RootControlLedger,
        stop: &AtomicBool,
    ) -> Result<(), BackupError> {
        self.recheck_inputs(ledger, true, stop)?;
        let original = self.file.as_ref().ok_or_else(invalid)?;
        let stamp = self.writer_stamp.as_ref().ok_or_else(invalid)?;
        let readback = self.readback.as_ref().ok_or_else(invalid)?;
        pay(&self.scan, self.raw.len(), 1)?;
        if Stamp::read(&original.metadata()?) != *stamp
            || readback.stamp != *stamp
            || readback.raw != self.raw
        {
            return Err(invalid());
        }
        readback.recheck(self.input.install, OBSERVATION_NAME, &self.scan)?;
        pay(&self.scan, self.raw.len() + 64, 1)?;
        let digest: [u8; 32] = Sha256::digest(&self.raw).into();
        if digest != readback.digest || Stamp::read(&original.metadata()?) != *stamp {
            return Err(invalid());
        }
        ledger.boundary(stop)
    }
    fn send_ack(
        &mut self,
        ledger: &mut RootControlLedger,
        socket: &UnixStream,
        stop: &AtomicBool,
    ) -> Result<(), BackupError> {
        ledger.begin()?;
        if self.failed || !self.durable || self.ack_attempted {
            self.failed = true;
            return Err(invalid());
        }
        self.failed = true;
        self.ack_attempted = true;
        // A lost/late/partial ACK never becomes send success by later readback.
        self.recheck_published(ledger, stop)?;
        pay(&self.scan, CONTROL_CAP, 16)?;
        let data = self.data.as_ref().ok_or_else(invalid)?;
        let notice = ledger.installed.as_ref().ok_or_else(invalid)?;
        data.matches_notice(notice)?;
        let reply = RootControlData::BrokerInstallObserved {
            schema_version: 1,
            job_id: ledger.job.clone(),
            root_cookie: ledger.cookie.clone(),
            sequence: ledger.next[1][1],
            backup_id: data.backup_id.to_string(),
            request_id: data.request_id.to_string(),
            binding_id: data.binding_id.to_string(),
            epoch_id: data.epoch_id.to_string(),
            descriptor_sha256: data.descriptor_sha256.clone(),
            observation_sha256: hex::encode(self.readback.as_ref().ok_or_else(invalid)?.digest),
        };
        ledger.send_inner(RootChannel::Capture, socket, &reply, stop)?;
        self.failed = false;
        ledger.failed = false;
        Ok(())
    }
}

fn install_roster(
    parent: &BackupDir,
    backup: &str,
    observed: bool,
    scan: &SharedJournalBudget,
) -> Result<(), BackupError> {
    let maximum = if observed { 5 } else { 4 };
    // Capped iterator includes its overflow entry, dots/EOF/FD and every
    // bounded NAME_MAX allocation before enumeration. No per-array allowance.
    pay(
        scan,
        64 * 1024 + (maximum + 1) * 255 * 2 + 4096,
        maximum * 4 + 12,
    )?;
    let mut expected = vec![
        "generation-binding.json".to_owned(),
        format!("inspection-{backup}.json"),
        format!("isolation-{backup}.json"),
        format!("isolation-{backup}.lock"),
    ];
    if observed {
        expected.push(OBSERVATION_NAME.to_owned());
    }
    expected.sort_unstable();
    let mut actual = parent.list_bounded(maximum)?;
    actual.sort_unstable();
    if actual != expected {
        return Err(invalid());
    }
    for name in actual {
        pay(scan, name.len() + 1, 1)?;
        if parent.kind(&name)? != learning_assets::backup_fs::BackupEntryKind::File {
            return Err(invalid());
        }
    }
    Ok(())
}

// Count typed serialization without building a Value/tree/string first. Every
// count/encoding write is prepaid, cap+1 rejects before canonical allocation,
// then the entire canonical working set is prepaid on the same original scan.
struct ObservationCounter<'a> {
    bytes: usize,
    scan: &'a SharedJournalBudget,
}
impl std::io::Write for ObservationCounter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        root_runtime::scan_charge(self.scan, bytes.len() as u64, 1)?;
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .filter(|n| *n <= RECORD_CAP)
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "C11 record cap")
            })?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn encode_observation(
    data: &InstallCutoverObservationData,
    scan: &SharedJournalBudget,
) -> Result<Vec<u8>, BackupError> {
    pay(scan, 4096, 1)?;
    let mut counter = ObservationCounter { bytes: 0, scan };
    serde_json::to_writer(&mut counter, data)?;
    if counter.bytes == 0 {
        return Err(invalid());
    }
    pay(scan, counter.bytes.checked_mul(64).ok_or_else(invalid)?, 0)?;
    let raw = registry::canonical(data)?;
    if raw.len() != counter.bytes || raw.len() > RECORD_CAP {
        return Err(invalid());
    }
    Ok(raw)
}

#[cfg(test)]
mod data_tests {
    use super::*;

    // Complete observation-shaped DATA, with no Docker calls or authority
    // constructors. The foreign container below is deliberately outside the
    // Compose project so alias rejection cannot depend on a project filter.
    fn projection_fixture() -> (HostCutoverData, InstalledMountData) {
        let destinations = [
            "/var/lib/postgresql",
            "/var/lib/knowweave-source",
            "/var/lib/knowweave-source/assets",
            "/var/lib/knowweave-c4/registry",
            InstalledSnapshotBinding::COMPANION_PATH,
            InstalledSnapshotBinding::EXPORT_PATH,
            "/run/kwc4/sfx",
            "/var/lib/knowweave-c4/b1-source-observation",
            "/opt/knowweave-c4/b1-capture",
            "/usr/bin/docker",
            "/var/run/docker.sock",
            "/var/run/postgresql",
        ];
        let mut mounts: Vec<MountProjection> = destinations
            .into_iter()
            .enumerate()
            .map(|(index, destination)| {
                let dedicated = destination == InstalledSnapshotBinding::COMPANION_PATH
                    || destination == InstalledSnapshotBinding::EXPORT_PATH;
                MountProjection {
                    source: format!("/data/fixture-{index}"),
                    destination: destination.into(),
                    rw: ![2, 7, 8, 9, 10].contains(&index),
                    kind: if dedicated { "volume" } else { "bind" }.into(),
                    name: dedicated.then(|| format!("fixture-volume-{index}")),
                }
            })
            .collect();
        mounts.sort_unstable_by(|a, b| a.destination.cmp(&b.destination));
        let rows: Vec<serde_json::Value> = mounts
            .iter()
            .map(|mount| {
                serde_json::json!({
                    "destination": mount.destination, "source": mount.source,
                    "kind": mount.kind, "rw": mount.rw, "name": mount.name.as_deref().unwrap_or("")
                })
            })
            .collect();
        let mount_digest = hex::encode(Sha256::digest(registry::canonical(&rows).unwrap()));
        let raw_assets = mounts
            .iter()
            .find(|mount| mount.destination == "/var/lib/knowweave-source/assets")
            .unwrap();
        let assets_mount = ActualAssetsMount {
            kind: raw_assets.kind.clone(),
            source: raw_assets.source.clone(),
            destination: raw_assets.destination.clone(),
            rw: raw_assets.rw,
            name: ActualOptionalString::Missing,
            driver: ActualOptionalString::Missing,
            mode: ActualOptionalString::Missing,
            propagation: ActualOptionalString::Missing,
        };
        let host = HostCutoverData {
            container_id: "11".repeat(32),
            image_sha256: format!("sha256:{}", "22".repeat(32)),
            export_mount: mounts
                .iter()
                .find(|mount| mount.destination == InstalledSnapshotBinding::EXPORT_PATH)
                .unwrap()
                .source
                .clone(),
            journal_mount: mounts
                .iter()
                .find(|mount| mount.destination == InstalledSnapshotBinding::COMPANION_PATH)
                .unwrap()
                .source
                .clone(),
            assets_mount,
            legacy_producers: Vec::new(),
            complete_container_projection: vec![ContainerProjection {
                container_id: "11".repeat(32),
                image_sha256: format!("sha256:{}", "22".repeat(32)),
                running: true,
                process_id: 41,
                started_at: "DATA_ONLY".into(),
                restart_count: 0,
                project: Some("data-only-project".into()),
                service: Some("pg".into()),
                mounts,
            }],
            daemon_id: "DATA_ONLY".into(),
            mountinfo_sha256: "33".repeat(32),
            docker_cli_sha256: "44".repeat(32),
        };
        let installed = InstalledMountData {
            container_id: host.container_id.clone(),
            image_sha256: host.image_sha256.clone(),
            daemon_id: host.daemon_id.clone(),
            compose_project: "data-only-project".into(),
            assets_inside: "/var/lib/knowweave-source/assets".into(),
            mount_digest,
        };
        (host, installed)
    }

    #[test]
    fn all_host_projection_rejects_rw_alias_outside_compose_project() {
        let (mut host, installed) = projection_fixture();
        check_projection(&host, &installed, &SharedJournalBudget::default()).unwrap();
        host.complete_container_projection
            .push(ContainerProjection {
                container_id: "ff".repeat(32),
                image_sha256: format!("sha256:{}", "aa".repeat(32)),
                running: true,
                process_id: 42,
                started_at: "DATA_ONLY".into(),
                restart_count: 0,
                project: Some("other-project".into()),
                service: None,
                mounts: vec![MountProjection {
                    source: format!("{}/child", host.journal_mount),
                    destination: "/alias".into(),
                    rw: true,
                    kind: "bind".into(),
                    name: None,
                }],
            });
        assert!(check_projection(&host, &installed, &SharedJournalBudget::default()).is_err());
        host.complete_container_projection[1].mounts[0].rw = false;
        check_projection(&host, &installed, &SharedJournalBudget::default()).unwrap();
    }

    #[test]
    fn before_after_rejects_raw_assets_presence_drift() {
        let (before, installed) = projection_fixture();
        let (mut after, _) = projection_fixture();
        same_before_after(&before, &after, &installed, &SharedJournalBudget::default()).unwrap();
        after.assets_mount.name = ActualOptionalString::Present(None);
        // Both normalize to the same bind name. Actual raw metadata equality
        // is nevertheless required across the original observations.
        check_projection(&after, &installed, &SharedJournalBudget::default()).unwrap();
        assert!(
            same_before_after(&before, &after, &installed, &SharedJournalBudget::default())
                .is_err()
        );
    }

    #[test]
    fn actual_assets_preserves_absent_and_null_and_rejects_unknown_properties() {
        let meter = SharedJournalBudget::default();
        let absent = br#"{"Destination":"/assets","RW":false,"Source":"/source","Type":"bind"}"#;
        let null =
            br#"{"Destination":"/assets","Name":null,"RW":false,"Source":"/source","Type":"bind"}"#;
        let a: ActualAssetsMount = parse(absent, RECORD_CAP, &meter).unwrap();
        let n: ActualAssetsMount = parse(null, RECORD_CAP, &meter).unwrap();
        assert!(matches!(&a.name, ActualOptionalString::Missing));
        assert!(matches!(&n.name, ActualOptionalString::Present(None)));
        assert_eq!(registry::canonical(&a).unwrap().as_slice(), &absent[..]);
        assert_eq!(registry::canonical(&n).unwrap().as_slice(), &null[..]);
        let unknown = br#"{"Destination":"/assets","RW":false,"Secret":"never accepted","Source":"/source","Type":"bind"}"#;
        assert!(parse::<ActualAssetsMount>(unknown, RECORD_CAP, &meter).is_err());
    }

    // A wire/Descriptor DATA fixture only. It constructs no process, Root job,
    // broker, installed binding, journal, session or cut authority.
    fn fixture() -> (BrokerInstalledData, Vec<u8>, Vec<u8>) {
        let binding = Uuid::parse_str("10000000-0000-4000-8000-000000000001").unwrap();
        let epoch = Uuid::parse_str("10000000-0000-4000-8000-000000000002").unwrap();
        let descriptor = serde_json::json!({
            "format_version":2,"capability":"snapshot_effect_epoch_v2","storage_encoding":"root_frame_v1",
            "deployment_id":binding,"epoch_id":epoch,"binding_id":binding,"source_group_id":binding,
            "source_application_commit":"1111111111111111111111111111111111111111",
            "source_build_sha256":"11".repeat(32),"broker_binary_sha256":"22".repeat(32),
            "broker_pid":4,"broker_uid":0,"broker_gid":0,"broker_binary_dev":1,"broker_binary_ino":2,"broker_binary_bytes":3,
            "source_database":"DATA_ONLY","source_database_oid":1,"source_system_identifier":"1","source_postmaster_start":"DATA_ONLY",
            "companion_root_dev":1,"companion_root_ino":2,"export_root_dev":1,"export_root_ino":3,
            "journal_root_dev":1,"journal_root_ino":4,"effect_lock_dev":1,"effect_lock_ino":5,
            "initial_namespace_sha256":"33".repeat(32),"writer_contract_sha256":"44".repeat(32),
            "work_socket_dev":1,"work_socket_ino":6,"management_socket_dev":1,"management_socket_ino":7
        });
        let payload = serde_json::to_vec(&descriptor).unwrap();
        let payload_hash: [u8; 32] = Sha256::digest(&payload).into();
        let notice = BrokerInstalledData {
            schema_version: 1,
            kind: "BrokerInstalled".into(),
            job_id: binding,
            root_cookie: "55".repeat(32),
            sequence: 0,
            backup_id: epoch,
            request_id: binding,
            binding_id: binding,
            epoch_id: epoch,
            descriptor_sha256: hex::encode(payload_hash),
        };
        let mut header = b"KWSJEPO2".to_vec();
        header.extend_from_slice(epoch.as_bytes());
        header.extend_from_slice(binding.as_bytes());
        let mut frame = b"KWSJBYT2".to_vec();
        frame.extend_from_slice(&1_u64.to_be_bytes());
        frame.extend_from_slice(epoch.as_bytes());
        frame.extend_from_slice(binding.as_bytes());
        frame.extend_from_slice(&Sha256::digest(&header));
        frame.extend_from_slice(&payload_hash);
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
        frame.extend_from_slice(&payload);
        (notice, header, frame)
    }

    #[test]
    fn genesis_descriptor_header_and_whole_frame_hashes_are_distinct_domains() {
        let (notice, header, frame) = fixture();
        let (_, payload_hash, header_hash, frame_hash) =
            read_genesis_bytes(&header, &frame, &notice, &SharedJournalBudget::default()).unwrap();
        assert_eq!(payload_hash, notice.descriptor_sha256);
        assert_eq!(header_hash, hex::encode(Sha256::digest(&header)));
        assert_eq!(frame_hash, hex::encode(Sha256::digest(&frame)));
        assert_ne!(payload_hash, frame_hash);
        let mut altered = frame.clone();
        altered[15] = 2;
        assert!(
            read_genesis_bytes(&header, &altered, &notice, &SharedJournalBudget::default())
                .is_err()
        );
        let mut altered = frame;
        altered[48] ^= 1;
        assert!(
            read_genesis_bytes(&header, &altered, &notice, &SharedJournalBudget::default())
                .is_err()
        );
    }

    #[test]
    fn genesis_rejects_truncated_header_payload_and_extra_frame_bytes() {
        let (notice, header, frame) = fixture();
        for end in 0..header.len() {
            assert!(
                read_genesis_bytes(
                    &header[..end],
                    &frame,
                    &notice,
                    &SharedJournalBudget::default()
                )
                .is_err()
            );
        }
        for end in [0, 7, 15, 47, 79, 111, 119, 120, frame.len() - 1] {
            assert!(
                read_genesis_bytes(
                    &header,
                    &frame[..end],
                    &notice,
                    &SharedJournalBudget::default()
                )
                .is_err()
            );
        }
        let mut extra = frame;
        extra.push(0);
        assert!(
            read_genesis_bytes(&header, &extra, &notice, &SharedJournalBudget::default()).is_err()
        );
    }

    #[test]
    fn bounded_data_parse_preserves_original_exhausted_meter() {
        let meter = SharedJournalBudget::default();
        meter.charge(512 * 1024 * 1024 - 8, 0).unwrap();
        let raw = br#"{"Destination":"/assets","RW":false,"Source":"/source","Type":"bind"}"#;
        assert!(parse::<ActualAssetsMount>(raw, RECORD_CAP, &meter).is_err());
        assert!(meter.usage().unwrap().exhausted);
        assert!(parse::<ActualAssetsMount>(raw, RECORD_CAP, &meter.clone()).is_err());
    }

    // Only protocol/record DATA fixtures. These tests never create an
    // OriginalRootJob, authenticate a channel, install a broker or bypass a
    // Root:Root file check with fabricated metadata or a success callback.
    fn control_value(kind: &str, sequence: u64) -> serde_json::Value {
        let mut value = serde_json::json!({
            "schema_version": 1, "kind": kind, "sequence": sequence,
            "job_id": "10000000-0000-4000-8000-000000000001", "root_cookie": "55".repeat(32),
        });
        let object = value.as_object_mut().unwrap();
        let backup = "10000000-0000-4000-8000-000000000002";
        let request = match kind {
            "CaptureInit" => "10000000-0000-4000-8000-000000000003",
            "RequestCaptureProof" | "CaptureProofBound" => "10000000-0000-4000-8000-000000000005",
            _ => "10000000-0000-4000-8000-000000000004",
        };
        let fields: &[(&str, &str)] = match kind {
            "SourceBootstrap" | "CaptureBootstrap" => &[("source_start_sha256", "aa")],
            "SpawnCapture" => &[],
            "CaptureInit" => &[
                ("backup_id", backup),
                ("request_id", request),
                ("capture_nonce", "bb"),
            ],
            "CaptureStartBound" => &[("backup_id", backup), ("capture_start_sha256", "cc")],
            "RequestInstallProof" | "RequestCaptureProof" => {
                &[("backup_id", backup), ("request_id", request)]
            }
            "InstallProofBound" | "CaptureProofBound" => &[
                ("backup_id", backup),
                ("request_id", request),
                ("generation_binding_sha256", "dd"),
            ],
            "BrokerInstalled" | "BrokerInstallObserved" => &[
                ("backup_id", backup),
                ("request_id", request),
                ("binding_id", "10000000-0000-4000-8000-000000000006"),
                ("epoch_id", "10000000-0000-4000-8000-000000000007"),
                ("descriptor_sha256", "ee"),
            ],
            "CapturedDataReady" | "SettleCapture" => {
                &[("backup_id", backup), ("captured_data_sha256", "ff")]
            }
            "CaptureSettled" => &[("backup_id", backup), ("capture_settlement_sha256", "ab")],
            "Abort" => &[("reason_code", "InputShape")],
            _ => &[],
        };
        for (key, value) in fields {
            object.insert(
                (*key).into(),
                serde_json::Value::String(if value.len() == 2 {
                    value.repeat(32)
                } else {
                    (*value).into()
                }),
            );
        }
        if kind == "BrokerInstallObserved" {
            object.insert(
                "observation_sha256".into(),
                serde_json::Value::String("ac".repeat(32)),
            );
        }
        value
    }
    fn control_data(kind: &str, sequence: u64) -> RootControlData {
        serde_json::from_value(control_value(kind, sequence)).unwrap()
    }
    fn control_ledger() -> RootControlLedger {
        RootControlLedger::data_only(
            SharedJournalBudget::default(),
            "10000000-0000-4000-8000-000000000001",
            &"55".repeat(32),
            Instant::now() + Duration::from_secs(1200),
        )
        .unwrap()
    }
    fn accept_at(
        ledger: &mut RootControlLedger,
        value: serde_json::Value,
        channel: RootChannel,
        direction: RootDirection,
        now: Instant,
    ) -> Result<(), BackupError> {
        ledger.begin()?;
        pay(&ledger.scan, CONTROL_CAP * 64, 64)?;
        let raw = registry::canonical(&value)?;
        ledger.wire.prefix(&ledger.scan)?;
        ledger.wire.body(raw.len(), &ledger.scan)?;
        let data: RootControlData = parse(&raw, CONTROL_CAP, &ledger.scan)?;
        ledger.observe(&data, channel, direction, now)?;
        ledger.failed = false;
        Ok(())
    }
    fn accept(
        ledger: &mut RootControlLedger,
        kind: &str,
        sequence: u64,
        channel: RootChannel,
        direction: RootDirection,
    ) -> Result<(), BackupError> {
        accept_at(
            ledger,
            control_value(kind, sequence),
            channel,
            direction,
            Instant::now(),
        )
    }
    fn through_start(ledger: &mut RootControlLedger) {
        use RootChannel::{Capture as C, Source as S};
        use RootDirection::{Receive as R, Send as W};
        for (kind, seq, channel, direction) in [
            ("SourceBootstrap", 0, S, W),
            ("SpawnCapture", 0, S, R),
            ("CaptureBootstrap", 0, C, W),
            ("CaptureInit", 0, C, R),
            ("CaptureStartBound", 1, C, W),
        ] {
            accept(ledger, kind, seq, channel, direction).unwrap();
        }
    }
    fn through_installed(ledger: &mut RootControlLedger) {
        through_start(ledger);
        accept(
            ledger,
            "RequestInstallProof",
            1,
            RootChannel::Capture,
            RootDirection::Receive,
        )
        .unwrap();
        accept(
            ledger,
            "InstallProofBound",
            2,
            RootChannel::Capture,
            RootDirection::Send,
        )
        .unwrap();
        accept(
            ledger,
            "BrokerInstalled",
            2,
            RootChannel::Capture,
            RootDirection::Receive,
        )
        .unwrap();
    }
    fn reserve_fixture(ledger: &mut RootControlLedger, length: usize) -> Result<(), BackupError> {
        ledger.begin()?;
        ledger.wire.prefix(&ledger.scan)?;
        ledger.wire.body(length, &ledger.scan)?;
        ledger.failed = false;
        Ok(())
    }

    #[test]
    fn root_wire15_matches_exact_typed_canonical_field_sets() {
        let kinds = [
            "SourceBootstrap",
            "SpawnCapture",
            "CaptureBootstrap",
            "CaptureInit",
            "CaptureStartBound",
            "RequestInstallProof",
            "InstallProofBound",
            "BrokerInstalled",
            "BrokerInstallObserved",
            "RequestCaptureProof",
            "CaptureProofBound",
            "CapturedDataReady",
            "SettleCapture",
            "CaptureSettled",
            "Abort",
        ];
        assert_eq!(kinds.len(), 15);
        for kind in kinds {
            let value = control_value(kind, 0);
            let raw = registry::canonical(&value).unwrap();
            let parsed: RootControlData =
                parse(&raw, CONTROL_CAP, &SharedJournalBudget::default()).unwrap();
            parsed.validate().unwrap();
            assert_eq!(parsed.kind(), kind);
            assert_eq!(registry::canonical(&parsed).unwrap(), raw);
            let mut extra = value.clone();
            extra["extra"] = serde_json::json!(false);
            assert!(
                parse::<RootControlData>(
                    &registry::canonical(&extra).unwrap(),
                    CONTROL_CAP,
                    &SharedJournalBudget::default()
                )
                .is_err()
            );
            for key in value.as_object().unwrap().keys() {
                let mut missing = value.clone();
                missing.as_object_mut().unwrap().remove(key);
                assert!(
                    parse::<RootControlData>(
                        &registry::canonical(&missing).unwrap(),
                        CONTROL_CAP,
                        &SharedJournalBudget::default()
                    )
                    .is_err()
                );
            }
        }
    }
    #[test]
    fn root_wire_rejects_duplicate_unknown_noncanonical_and_bad_scalar() {
        let raw = registry::canonical(&control_value("SourceBootstrap", 0)).unwrap();
        let tail = std::str::from_utf8(&raw[1..]).unwrap();
        for key in ["kind", "schema_version", "job_id"] {
            let duplicate = format!(
                "{{\"{key}\":{},{}",
                control_value("SourceBootstrap", 0)[key],
                tail
            );
            assert!(
                parse::<RootControlData>(
                    duplicate.as_bytes(),
                    CONTROL_CAP,
                    &SharedJournalBudget::default()
                )
                .is_err()
            );
        }
        let mut noncanonical = raw.clone();
        noncanonical.push(b' ');
        assert!(
            parse::<RootControlData>(&noncanonical, CONTROL_CAP, &SharedJournalBudget::default())
                .is_err()
        );
        for (key, value) in [
            ("kind", serde_json::json!("NewRootAction")),
            ("root_cookie", serde_json::json!("A".repeat(64))),
            (
                "job_id",
                serde_json::json!("10000000-0000-4000-C000-000000000001"),
            ),
        ] {
            let mut input = control_value("SourceBootstrap", 0);
            input[key] = value;
            if let Ok(parsed) = parse::<RootControlData>(
                &registry::canonical(&input).unwrap(),
                CONTROL_CAP,
                &SharedJournalBudget::default(),
            ) {
                assert!(parsed.validate().is_err());
            }
        }
    }
    #[test]
    fn root_shared_budget_counts_both_endpoint_borrows_and_prefix_boundary() {
        let mut ledger = control_ledger();
        {
            let source = &mut ledger;
            for _ in 0..32 {
                reserve_fixture(source, CONTROL_CAP).unwrap();
            }
        }
        {
            let capture = &mut ledger;
            for _ in 0..32 {
                reserve_fixture(capture, CONTROL_CAP).unwrap();
            }
        }
        assert_eq!((ledger.wire.frames, ledger.wire.bytes), (64, 262_400));
        let usage = ledger.scan.usage().unwrap();
        assert!(reserve_fixture(&mut ledger, 1).is_err());
        assert!(ledger.failed);
        assert!(reserve_fixture(&mut ledger, 1).is_err());
        assert_eq!(
            ledger.scan.usage().unwrap().charged_bytes,
            usage.charged_bytes
        );
        assert_eq!((ledger.wire.frames, ledger.wire.bytes), (64, 262_400));
    }
    #[test]
    fn root_frame_length_failure_and_original_scan_exhaustion_are_sticky() {
        for length in [0, CONTROL_CAP + 1] {
            let mut ledger = control_ledger();
            assert!(reserve_fixture(&mut ledger, length).is_err());
            assert_eq!(ledger.wire.frames, 1);
            assert!(reserve_fixture(&mut ledger, 1).is_err());
        }
        let mut ledger = control_ledger();
        let clone = ledger.scan.clone();
        let used = clone.usage().unwrap().charged_bytes;
        clone.charge(512 * 1024 * 1024 - used - 3, 0).unwrap();
        assert!(reserve_fixture(&mut ledger, 1).is_err());
        assert!(clone.usage().unwrap().exhausted);
        assert!(ledger.scan.shares_meter_with(&clone));
        assert!(reserve_fixture(&mut ledger, 1).is_err());
    }
    #[test]
    fn root_wrong_sequence_direction_and_channel_poison_both_ends() {
        use RootChannel::{Capture as C, Source as S};
        use RootDirection::{Receive as R, Send as W};
        for (kind, seq, channel, direction) in [
            ("SourceBootstrap", 1, S, W),
            ("SourceBootstrap", 0, S, R),
            ("SourceBootstrap", 0, C, W),
        ] {
            let mut ledger = control_ledger();
            assert!(accept(&mut ledger, kind, seq, channel, direction).is_err());
            assert!(accept(&mut ledger, "SourceBootstrap", 0, S, W).is_err());
            assert!(ledger.failed);
        }
    }
    #[test]
    fn root_request_reuse_refresh_and_installed_correlation_fail_permanently() {
        use RootChannel::Capture as C;
        use RootDirection::{Receive as R, Send as W};
        let mut ledger = control_ledger();
        through_start(&mut ledger);
        let mut reused = control_value("RequestInstallProof", 1);
        reused["request_id"] = control_value("CaptureInit", 0)["request_id"].clone();
        assert!(accept_at(&mut ledger, reused, C, R, Instant::now()).is_err());
        assert!(accept(&mut ledger, "RequestInstallProof", 1, C, R).is_err());
        let mut ledger = control_ledger();
        through_start(&mut ledger);
        accept(&mut ledger, "RequestInstallProof", 1, C, R).unwrap();
        let original_deadline = ledger.pending;
        assert!(accept(&mut ledger, "RequestInstallProof", 2, C, R).is_err());
        assert_eq!(ledger.pending, original_deadline);
        assert!(accept(&mut ledger, "InstallProofBound", 2, C, W).is_err());
        let mut ledger = control_ledger();
        through_start(&mut ledger);
        accept(&mut ledger, "RequestInstallProof", 1, C, R).unwrap();
        accept(&mut ledger, "InstallProofBound", 2, C, W).unwrap();
        let mut wrong = control_value("BrokerInstalled", 2);
        wrong["request_id"] = control_value("CaptureInit", 0)["request_id"].clone();
        assert!(accept_at(&mut ledger, wrong, C, R, Instant::now()).is_err());
    }
    #[test]
    fn root_total_and_original_request_deadlines_cannot_be_renewed() {
        let mut ledger = control_ledger();
        through_start(&mut ledger);
        accept(
            &mut ledger,
            "RequestInstallProof",
            1,
            RootChannel::Capture,
            RootDirection::Receive,
        )
        .unwrap();
        let deadline = ledger.pending.unwrap();
        let total = ledger.total;
        assert!(
            accept_at(
                &mut ledger,
                control_value("InstallProofBound", 2),
                RootChannel::Capture,
                RootDirection::Send,
                deadline
            )
            .is_err()
        );
        assert!(ledger.failed);
        assert_eq!(ledger.total, total);
        assert_eq!(ledger.pending, Some(deadline));
        assert!(
            accept(
                &mut ledger,
                "InstallProofBound",
                2,
                RootChannel::Capture,
                RootDirection::Send
            )
            .is_err()
        );
    }
    #[test]
    fn root_ordinary_sender_cannot_emit_ack_without_durable_publication() {
        use std::io::Read;
        let mut ledger = control_ledger();
        through_installed(&mut ledger);
        let (socket, mut peer) = UnixStream::pair().unwrap();
        peer.set_nonblocking(true).unwrap();
        assert!(
            ledger
                .send_data(
                    RootChannel::Capture,
                    &socket,
                    &control_data("BrokerInstallObserved", 3),
                    &AtomicBool::new(false)
                )
                .is_err()
        );
        let mut byte = [0];
        assert_eq!(
            peer.read(&mut byte).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert!(ledger.failed);
        assert_eq!(ledger.step, 8);
    }
    #[test]
    fn root_ack_wrong_order_and_duplicate_notice_cannot_advance() {
        let mut ledger = control_ledger();
        through_start(&mut ledger);
        assert!(
            accept(
                &mut ledger,
                "BrokerInstallObserved",
                2,
                RootChannel::Capture,
                RootDirection::Send
            )
            .is_err()
        );
        let mut ledger = control_ledger();
        through_installed(&mut ledger);
        assert!(
            accept(
                &mut ledger,
                "BrokerInstalled",
                3,
                RootChannel::Capture,
                RootDirection::Receive
            )
            .is_err()
        );
        assert!(
            accept(
                &mut ledger,
                "BrokerInstallObserved",
                3,
                RootChannel::Capture,
                RootDirection::Send
            )
            .is_err()
        );
    }
    #[test]
    fn root_native_socketpair_reads_frame_and_counts_shared_wire_bytes() {
        use std::io::Write;
        let mut ledger = control_ledger();
        accept(
            &mut ledger,
            "SourceBootstrap",
            0,
            RootChannel::Source,
            RootDirection::Send,
        )
        .unwrap();
        let raw = registry::canonical(&control_value("SpawnCapture", 0)).unwrap();
        let (socket, mut peer) = UnixStream::pair().unwrap();
        peer.write_all(&(raw.len() as u32).to_be_bytes()).unwrap();
        peer.write_all(&raw).unwrap();
        let previous = ledger.wire.bytes;
        let data = ledger
            .receive_data(RootChannel::Source, &socket, &AtomicBool::new(false))
            .unwrap();
        assert_eq!(data.kind(), "SpawnCapture");
        assert_eq!(ledger.wire.bytes - previous, (raw.len() + 4) as u64);
        assert_eq!(ledger.wire.frames, 2);
    }
    #[test]
    fn root_native_short_prefix_and_eof_burn_original_ledger() {
        use std::io::Write;
        let mut ledger = control_ledger();
        accept(
            &mut ledger,
            "SourceBootstrap",
            0,
            RootChannel::Source,
            RootDirection::Send,
        )
        .unwrap();
        let (socket, mut peer) = UnixStream::pair().unwrap();
        peer.write_all(&[0, 0]).unwrap();
        peer.shutdown(std::net::Shutdown::Write).unwrap();
        assert!(
            ledger
                .receive_data(RootChannel::Source, &socket, &AtomicBool::new(false))
                .is_err()
        );
        assert!(ledger.failed);
        assert!(
            ledger
                .receive_data(RootChannel::Capture, &socket, &AtomicBool::new(false))
                .is_err()
        );
    }
    #[test]
    fn root_native_extra_byte_and_cancellation_are_sticky() {
        use std::io::Write;
        let mut ledger = control_ledger();
        let (socket, mut peer) = UnixStream::pair().unwrap();
        peer.write_all(b"x").unwrap();
        assert!(
            ledger
                .require_idle_no_extra(&socket, &AtomicBool::new(false))
                .is_err()
        );
        assert!(
            ledger
                .require_idle_no_extra(&socket, &AtomicBool::new(false))
                .is_err()
        );
        let mut ledger = control_ledger();
        assert!(
            ledger
                .receive_data(RootChannel::Source, &socket, &AtomicBool::new(true))
                .is_err()
        );
        assert!(
            ledger
                .send_data(
                    RootChannel::Source,
                    &socket,
                    &control_data("SourceBootstrap", 0),
                    &AtomicBool::new(false)
                )
                .is_err()
        );
    }
    fn observation_data() -> InstallCutoverObservationData {
        let (notice, header, frame) = fixture();
        let (_, descriptor, epoch_header, genesis) =
            read_genesis_bytes(&header, &frame, &notice, &SharedJournalBudget::default()).unwrap();
        InstallCutoverObservationData {
            schema_version: 1,
            capability: "b1_install_cutover_observation_v1".into(),
            job_id: notice.job_id,
            backup_id: notice.backup_id,
            phase: "Install".into(),
            request_id: notice.request_id,
            binding_id: notice.binding_id,
            epoch_id: notice.epoch_id,
            generation_binding_sha256: "dd".repeat(32),
            descriptor_sha256: descriptor,
            epoch_header_sha256: epoch_header,
            genesis_frame_sha256: genesis,
            observed_unix_ms: 1,
            host_cutover: projection_fixture().0,
        }
    }
    #[test]
    fn c11_typed_encoder_preserves_complete_canonical_data_and_hash_domains() {
        let data = observation_data();
        let scan = SharedJournalBudget::default();
        let raw = encode_observation(&data, &scan).unwrap();
        let parsed = InstallCutoverObservationData::read_data(&raw, &scan).unwrap();
        assert_eq!(registry::canonical(&parsed).unwrap(), raw);
        assert!(parsed.host_cutover == data.host_cutover);
        assert_ne!(parsed.descriptor_sha256, parsed.genesis_frame_sha256);
        assert_ne!(parsed.epoch_header_sha256, parsed.genesis_frame_sha256);
        let (mut notice, _, _) = fixture();
        parsed.matches_notice(&notice).unwrap();
        notice.request_id = Uuid::parse_str("10000000-0000-4000-8000-000000000009").unwrap();
        assert!(parsed.matches_notice(&notice).is_err());
    }
    #[test]
    fn c11_typed_encoder_rejects_cap_before_canonical_tree_and_keeps_meter_exhausted() {
        let mut data = observation_data();
        data.host_cutover.daemon_id = "x".repeat(RECORD_CAP + 1);
        assert!(encode_observation(&data, &SharedJournalBudget::default()).is_err());
        let data = observation_data();
        let scan = SharedJournalBudget::default();
        scan.charge(512 * 1024 * 1024 - 1, 0).unwrap();
        assert!(encode_observation(&data, &scan).is_err());
        assert!(scan.usage().unwrap().exhausted);
        assert!(encode_observation(&data, &scan.clone()).is_err());
    }
}

// The only externally reachable surface in this leaf. All role selection,
// admission and original owners remain private and fixed to this process.
pub(super) fn run_fixed_root_launcher() -> Result<(), BackupError> {
    root_runtime::run()
}

mod root_runtime {
    use super::*;
    use std::{
        collections::{BTreeMap, BTreeSet, VecDeque},
        ffi::CString,
        io::Read,
        os::{
            fd::{FromRawFd, OwnedFd, RawFd},
            unix::process::CommandExt,
        },
        path::Path,
        process::{Child, Command, Stdio},
        sync::{Arc, Mutex, OnceLock},
        thread::JoinHandle,
    };
    type Result<T> = std::result::Result<T, BackupError>;
    use record as job;
    use record::Record;

    const OBSERVATION_ROOT: &str = "/var/lib/knowweave-c4/b1-source-observation";
    const CATALOG_PATH: &str = "/var/lib/knowweave-c4/b1/source-peer-execution.json";
    const PROFILE: &str = "task9-b1-limited-v1";
    const DOCKER_SOCKET: &str = "/run/docker.sock";
    const PG_IMAGE: &str = "postgres:18.6-bookworm@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d";
    const PG_DIGEST: &str =
        "sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d";
    static ATTEMPT: AtomicBool = AtomicBool::new(false);
    static STOP: AtomicBool = AtomicBool::new(false);
    static OBSERVATION_USED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    thread_local! {static OBSERVING:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};static CLEANING:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
    type NativeFilePool = BTreeMap<(u64, u64, u32), std::sync::Weak<File>>;
    type NamedParentPool = BTreeMap<String, (std::sync::Weak<File>, Stamp)>;
    static NATIVE_FILES: OnceLock<Mutex<NativeFilePool>> = OnceLock::new();
    static PARENTS: OnceLock<Mutex<NamedParentPool>> = OnceLock::new();

    pub(super) fn scan_charge(
        scan: &SharedJournalBudget,
        bytes: u64,
        entries: u64,
    ) -> std::io::Result<()> {
        if OBSERVING.with(std::cell::Cell::get)
            && OBSERVATION_USED
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                    n.checked_add(bytes).filter(|v| *v <= 64 * 1024 * 1024)
                })
                .is_err()
        {
            let _ = scan.charge(u64::MAX, u64::MAX);
            return Err(std::io::Error::other("B1 original observation budget"));
        }
        scan.charge(bytes, entries)
    }
    struct ObservationScope(bool);
    impl ObservationScope {
        fn enter() -> Self {
            Self(OBSERVING.with(|v| v.replace(true)))
        }
    }
    impl Drop for ObservationScope {
        fn drop(&mut self) {
            OBSERVING.with(|v| v.set(self.0));
        }
    }
    pub(super) fn cleanup_io() -> bool {
        CLEANING.with(std::cell::Cell::get)
    }
    struct CleanupScope(bool);
    impl CleanupScope {
        fn enter() -> Self {
            Self(CLEANING.with(|v| v.replace(true)))
        }
    }
    impl Drop for CleanupScope {
        fn drop(&mut self) {
            CLEANING.with(|v| v.set(self.0));
        }
    }

    // A private, field-for-field mirror of the actual DB record consumer.
    // These shapes are DATA; only the kernel/held stack below creates a job.
    mod record {
        use super::{BackupError, invalid};
        use serde::{Deserialize, Serialize};
        type Result<T> = std::result::Result<T, BackupError>;
        pub(super) trait Record: Serialize {
            fn validate(&self) -> Result<()>;
        }
        pub(super) fn hex(value: &str, length: usize) -> bool {
            value.len() == length
                && value
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }
        fn digest_bytes(value: &str) -> Result<[u8; 32]> {
            if !hex(value, 64) {
                return Err(invalid());
            }
            let mut out = [0; 32];
            for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
                let digit = |c: u8| if c <= b'9' { c - b'0' } else { c - b'a' + 10 };
                out[index] = digit(chunk[0]) * 16 + digit(chunk[1]);
            }
            Ok(out)
        }
        pub(super) fn uuid(value: &str, v4: bool) -> bool {
            uuid::Uuid::parse_str(value).is_ok_and(|id| {
                id.hyphenated().to_string() == value
                    && (!v4
                        || (id.get_version_num() == 4
                            && id.get_variant() == uuid::Variant::RFC4122))
            })
        }
        pub(super) fn path(value: &str) -> bool {
            value.starts_with('/')
                && value.len() <= 1024
                && !value.as_bytes().contains(&0)
                && value[1..]
                    .split('/')
                    .all(|s| !s.is_empty() && s != "." && s != "..")
        }
        fn namespace(value: &str, prefix: &str) -> bool {
            value
                .strip_prefix(prefix)
                .and_then(|v| v.strip_suffix(']'))
                .is_some_and(|n| {
                    !n.starts_with('0')
                        && n.parse::<u64>().is_ok_and(|v| v > 0 && v.to_string() == n)
                })
        }
        fn pattern(value: &str, pattern: &str) -> bool {
            match pattern {
                "^(learning-system-p0c4-[A-Za-z0-9_-]+|kwc4c-[0-9a-f]{32})$" => {
                    value
                        .strip_prefix("learning-system-p0c4-")
                        .is_some_and(|s| {
                            !s.is_empty()
                                && s.bytes()
                                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                        })
                        || value.strip_prefix("kwc4c-").is_some_and(|s| hex(s, 32))
                }
                "^/(?!/)(?!.*(?:^|/)\\.\\.?(?:/|$))[^\\x00]+$" => path(value),
                "^PGDATABASE=learning_backup_c4_task3_[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$" => {
                    value.strip_prefix("PGDATABASE=").is_some_and(database)
                }
                "^[0-9a-f]{40}$" => hex(value, 40),
                "^[0-9a-f]{64}$" => hex(value, 64),
                "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$" => {
                    uuid(value, true)
                }
                "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$" => {
                    uuid(value, false)
                }
                "^[1-9][0-9]{0,19}$" => {
                    !value.starts_with('0')
                        && value
                            .parse::<u64>()
                            .is_ok_and(|v| v > 0 && v.to_string() == value)
                }
                "^[A-Za-z0-9:_-]+$" => {
                    !value.is_empty()
                        && value
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b":_-".contains(&b))
                }
                "^learning_backup_c4_task3_[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$" => {
                    database(value)
                }
                "^mnt:\\[[1-9][0-9]*\\]$" => namespace(value, "mnt:["),
                "^pid:\\[[1-9][0-9]*\\]$" => namespace(value, "pid:["),
                "^sha256:[0-9a-f]{64}$" => {
                    value.strip_prefix("sha256:").is_some_and(|v| hex(v, 64))
                }
                _ => false,
            }
        }
        pub(super) fn database(value: &str) -> bool {
            value.len() <= 63
                && value
                    .strip_prefix("learning_backup_c4_task3_")
                    .is_some_and(|v| uuid(v, true))
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct CaptureExecRecipe {
            pub(super) schema_version: u64,
            pub(super) entrypoint: String,
            pub(super) arguments: Vec<String>,
            pub(super) wrapper_code: CaptureExecRecipeWrapperCode,
            pub(super) wrapper_libraries: Vec<CodeFile>,
            pub(super) final_capture_path: String,
            pub(super) same_exec_pid_start_no_fork: bool,
        }
        impl Record for CaptureExecRecipe {
            fn validate(&self) -> Result<()> {
                if self.schema_version == 1
            && self.entrypoint == "/usr/bin/env"
            && self.arguments.len() >= 9
            && self.arguments.len() <= 9
            && self.arguments.first().is_some_and(|item| item == "-i")
            && self.arguments.get(1).is_some_and(|item| item == "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
            && self.arguments.get(2).is_some_and(|item| item == "HOME=/root")
            && self.arguments.get(3).is_some_and(|item| item == "LANG=C")
            && self.arguments.get(4).is_some_and(|item| item == "PGHOST=/var/run/postgresql")
            && self.arguments.get(5).is_some_and(|item| item == "PGPORT=5432")
            && self.arguments.get(6).is_some_and(|item| item == "PGUSER=learning_admin")
            && self.arguments.get(7).is_some_and(|item| item.len() <= 74 && pattern(item, "^PGDATABASE=learning_backup_c4_task3_[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"))
            && self.arguments.get(8).is_some_and(|item| item == "/opt/knowweave-c4/b1-capture/knowweave-c4-b1-capture")
            && self.wrapper_code.validate().is_ok()
            && self.wrapper_libraries.len() <= 32
            && self.wrapper_libraries.iter().all(|item| item.validate().is_ok())
            && self.wrapper_libraries.windows(2).all(|w|w[0].path<w[1].path)
            && self.final_capture_path == "/opt/knowweave-c4/b1-capture/knowweave-c4-b1-capture"
            && self.same_exec_pid_start_no_fork { Ok(()) } else { Err(invalid()) }
            }
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct CaptureExecRecipeWrapperCode {
            pub(super) path: String,
            pub(super) stamp: FileStamp,
            pub(super) sha256: String,
        }
        impl Record for CaptureExecRecipeWrapperCode {
            fn validate(&self) -> Result<()> {
                if self.path == "/usr/bin/env"
                    && self.stamp.validate().is_ok()
                    && self.sha256.len() <= 64
                    && pattern(&self.sha256, "^[0-9a-f]{64}$")
                {
                    Ok(())
                } else {
                    Err(invalid())
                }
            }
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct InstalledRuntime {
            pub(super) compose_project: String,
            pub(super) pg_container_id: String,
            pub(super) pg_image_ref: String,
            pub(super) daemon_id: String,
            pub(super) internal_network_id: String,
            pub(super) capture_mount_recipe_sha256: String,
            pub(super) source_root_records_sha256: String,
            pub(super) pg_pid_namespace: String,
            pub(super) pg_mount_namespace: String,
            pub(super) postmaster_start_ticks: u64,
            pub(super) admitted_pg_image_id: String,
            pub(super) pg_image_digest: String,
        }
        impl Record for InstalledRuntime {
            fn validate(&self) -> Result<()> {
                if self.compose_project.len() <= 63
                    && pattern(
                        &self.compose_project,
                        "^(learning-system-p0c4-[A-Za-z0-9_-]+|kwc4c-[0-9a-f]{32})$",
                    )
                    && self.pg_container_id.len() <= 64
                    && pattern(&self.pg_container_id, "^[0-9a-f]{64}$")
                    && self.pg_image_ref
                        == "postgres:18.6-bookworm@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d"
                    && self.daemon_id.len() <= 128
                    && pattern(&self.daemon_id, "^[A-Za-z0-9:_-]+$")
                    && self.internal_network_id.len() <= 64
                    && pattern(&self.internal_network_id, "^[0-9a-f]{64}$")
                    && self.capture_mount_recipe_sha256.len() <= 64
                    && pattern(&self.capture_mount_recipe_sha256, "^[0-9a-f]{64}$")
                    && self.source_root_records_sha256.len() <= 64
                    && pattern(&self.source_root_records_sha256, "^[0-9a-f]{64}$")
                    && self.pg_pid_namespace.len() <= 64
                    && pattern(&self.pg_pid_namespace, "^pid:\\[[1-9][0-9]*\\]$")
                    && self.pg_mount_namespace.len() <= 64
                    && pattern(&self.pg_mount_namespace, "^mnt:\\[[1-9][0-9]*\\]$")
                    && self.postmaster_start_ticks >= 1
                    && self.admitted_pg_image_id.len() <= 71
                    && pattern(&self.admitted_pg_image_id, "^sha256:[0-9a-f]{64}$")
                    && self.pg_image_digest
                        == "sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d"
                {
                    Ok(())
                } else {
                    Err(invalid())
                }
            }
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct Hold {
            pub(super) schema_version: u64,
            pub(super) capability: String,
            pub(super) job_id: String,
            pub(super) phase: String,
            pub(super) reason_code: String,
            pub(super) retry_allowed: bool,
            pub(super) owner_settlement: String,
        }
        impl Record for Hold {
            fn validate(&self) -> Result<()> {
                if self.schema_version == 1
                    && self.capability == "b1_source_job_hold_v1"
                    && self.job_id.len() <= 36
                    && pattern(
                        &self.job_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && [
                        "Bootstrap",
                        "Install",
                        "Quiesce",
                        "Capture",
                        "Transfer",
                        "Settle",
                        "Final",
                    ]
                    .contains(&self.phase.as_str())
                    && [
                        "InputShape",
                        "ProcessChanged",
                        "RootOwnerLost",
                        "CookieMismatch",
                        "Phase",
                        "StaleInitialProof",
                        "Budget",
                        "ChildExit",
                        "UnexpectedEof",
                        "ExtraBytes",
                        "Deadline",
                        "SettlementUnconfirmed",
                    ]
                    .contains(&self.reason_code.as_str())
                    && !self.retry_allowed
                    && ["Unconfirmed", "ObservedStopped"].contains(&self.owner_settlement.as_str())
                {
                    Ok(())
                } else {
                    Err(invalid())
                }
            }
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct CaptureSettlement {
            pub(super) schema_version: u64,
            pub(super) capability: String,
            pub(super) job_id: String,
            pub(super) backup_id: String,
            pub(super) capture_start_sha256: String,
            pub(super) captured_data_sha256: String,
            pub(super) exec_id: String,
            pub(super) child_start_ticks: u64,
            pub(super) natural_exit_code: u64,
            pub(super) stdout_eof: bool,
            pub(super) stderr_eof: bool,
            pub(super) extra_bytes: bool,
            pub(super) owner_settled: bool,
            pub(super) stdout_sha256: String,
            pub(super) stderr_sha256: String,
        }
        impl Record for CaptureSettlement {
            fn validate(&self) -> Result<()> {
                if self.schema_version == 1
                    && self.capability == "b1_capture_natural_settlement_observation_v1"
                    && self.job_id.len() <= 36
                    && pattern(
                        &self.job_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && self.backup_id.len() <= 36
                    && pattern(
                        &self.backup_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && self.capture_start_sha256.len() <= 64
                    && pattern(&self.capture_start_sha256, "^[0-9a-f]{64}$")
                    && self.captured_data_sha256.len() <= 64
                    && pattern(&self.captured_data_sha256, "^[0-9a-f]{64}$")
                    && self.exec_id.len() <= 64
                    && pattern(&self.exec_id, "^[0-9a-f]{64}$")
                    && self.child_start_ticks >= 1
                    && self.natural_exit_code == 0
                    && self.stdout_eof
                    && self.stderr_eof
                    && !self.extra_bytes
                    && self.owner_settled
                    && self.stdout_sha256.len() <= 64
                    && pattern(&self.stdout_sha256, "^[0-9a-f]{64}$")
                    && self.stderr_sha256.len() <= 64
                    && pattern(&self.stderr_sha256, "^[0-9a-f]{64}$")
                {
                    Ok(())
                } else {
                    Err(invalid())
                }
            }
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct CapturedData {
            pub(super) schema_version: u64,
            pub(super) capability: String,
            pub(super) job_id: String,
            pub(super) backup_id: String,
            pub(super) capture_start_sha256: String,
            pub(super) capture_generation_sha256: String,
            pub(super) source_group_id: String,
            pub(super) source_registry_generation_sha256: String,
            pub(super) manifest_sha256: String,
            pub(super) captured_roster_sha256: String,
            pub(super) snapshot_effects_roster_sha256: String,
        }
        impl Record for CapturedData {
            fn validate(&self) -> Result<()> {
                if self.schema_version == 1
                    && self.capability == "b1_captured_input_data_only_v1"
                    && self.job_id.len() <= 36
                    && pattern(
                        &self.job_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && self.backup_id.len() <= 36
                    && pattern(
                        &self.backup_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && self.capture_start_sha256.len() <= 64
                    && pattern(&self.capture_start_sha256, "^[0-9a-f]{64}$")
                    && self.capture_generation_sha256.len() <= 64
                    && pattern(&self.capture_generation_sha256, "^[0-9a-f]{64}$")
                    && self.source_group_id.len() <= 36
                    && pattern(
                        &self.source_group_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && self.source_registry_generation_sha256.len() <= 64
                    && pattern(&self.source_registry_generation_sha256, "^[0-9a-f]{64}$")
                    && self.manifest_sha256.len() <= 64
                    && pattern(&self.manifest_sha256, "^[0-9a-f]{64}$")
                    && self.captured_roster_sha256.len() <= 64
                    && pattern(&self.captured_roster_sha256, "^[0-9a-f]{64}$")
                    && self.snapshot_effects_roster_sha256.len() <= 64
                    && pattern(&self.snapshot_effects_roster_sha256, "^[0-9a-f]{64}$")
                {
                    Ok(())
                } else {
                    Err(invalid())
                }
            }
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct GenerationBinding {
            pub(super) schema_version: u64,
            pub(super) capability: String,
            pub(super) job_id: String,
            pub(super) backup_id: String,
            pub(super) phase: String,
            pub(super) request_id: String,
            pub(super) capture_start_sha256: String,
            pub(super) isolation_bytes: u64,
            pub(super) isolation_sha256: String,
            pub(super) inspection_bytes: u64,
            pub(super) inspection_sha256: String,
            pub(super) lock_dev: u64,
            pub(super) lock_ino: u64,
            pub(super) observed_unix_ms: u64,
        }
        impl Record for GenerationBinding {
            fn validate(&self) -> Result<()> {
                if self.schema_version == 1
                    && self.capability == "b1_immutable_isolation_generation_v1"
                    && self.job_id.len() <= 36
                    && pattern(
                        &self.job_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && self.backup_id.len() <= 36
                    && pattern(
                        &self.backup_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && ["Install", "Capture"].contains(&self.phase.as_str())
                    && self.request_id.len() <= 36
                    && pattern(
                        &self.request_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && self.capture_start_sha256.len() <= 64
                    && pattern(&self.capture_start_sha256, "^[0-9a-f]{64}$")
                    && self.isolation_bytes >= 1
                    && self.isolation_bytes <= 4096
                    && self.isolation_sha256.len() <= 64
                    && pattern(&self.isolation_sha256, "^[0-9a-f]{64}$")
                    && self.inspection_bytes >= 1
                    && self.inspection_bytes <= 1048576
                    && self.inspection_sha256.len() <= 64
                    && pattern(&self.inspection_sha256, "^[0-9a-f]{64}$")
                    && self.lock_dev >= 1
                    && self.lock_ino >= 1
                    && self.observed_unix_ms >= 1
                {
                    Ok(())
                } else {
                    Err(invalid())
                }
            }
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct CaptureStart {
            pub(super) schema_version: u64,
            pub(super) job_id: String,
            pub(super) launch_id: String,
            pub(super) catalog_sha256: String,
            pub(super) boot_id: String,
            pub(super) root_cookie: String,
            pub(super) capability: String,
            pub(super) backup_id: String,
            pub(super) capture_nonce: String,
            pub(super) capture_request_id: String,
            pub(super) source_start_sha256: String,
            pub(super) container_id: String,
            pub(super) exec_id: String,
            pub(super) image_id: String,
            pub(super) mount_recipe_sha256: String,
            pub(super) capture: ProcessRef,
            pub(super) native: Native,
            pub(super) image_ref: String,
            pub(super) image_digest: String,
            pub(super) exec_process_config: CaptureStartExecProcessConfig,
            pub(super) capture_exec_recipe_sha256: String,
        }
        impl Record for CaptureStart {
            fn validate(&self) -> Result<()> {
                if self.schema_version == 1
                    && self.job_id.len() <= 36
                    && pattern(
                        &self.job_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && self.launch_id.len() <= 36
                    && pattern(
                        &self.launch_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && self.catalog_sha256.len() <= 64
                    && pattern(&self.catalog_sha256, "^[0-9a-f]{64}$")
                    && self.boot_id.len() <= 36
                    && pattern(
                        &self.boot_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$",
                    )
                    && self.root_cookie.len() <= 64
                    && pattern(&self.root_cookie, "^[0-9a-f]{64}$")
                    && self.capability == "b1_capture_process_start_v1"
                    && self.backup_id.len() <= 36
                    && pattern(
                        &self.backup_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && self.capture_nonce.len() <= 64
                    && pattern(&self.capture_nonce, "^[0-9a-f]{64}$")
                    && self.capture_request_id.len() <= 36
                    && pattern(
                        &self.capture_request_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && self.source_start_sha256.len() <= 64
                    && pattern(&self.source_start_sha256, "^[0-9a-f]{64}$")
                    && self.container_id.len() <= 64
                    && pattern(&self.container_id, "^[0-9a-f]{64}$")
                    && self.exec_id.len() <= 64
                    && pattern(&self.exec_id, "^[0-9a-f]{64}$")
                    && self.image_id.len() <= 71
                    && pattern(&self.image_id, "^sha256:[0-9a-f]{64}$")
                    && self.mount_recipe_sha256.len() <= 64
                    && pattern(&self.mount_recipe_sha256, "^[0-9a-f]{64}$")
                    && self.capture.validate().is_ok()
                    && self.native.validate().is_ok()
                    && self.image_ref
                        == "postgres:18.6-bookworm@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d"
                    && self.image_digest
                        == "sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d"
                    && self.exec_process_config.validate().is_ok()
                    && self.capture_exec_recipe_sha256.len() <= 64
                    && pattern(&self.capture_exec_recipe_sha256, "^[0-9a-f]{64}$")
                {
                    Ok(())
                } else {
                    Err(invalid())
                }
            }
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct CaptureStartExecProcessConfig {
            pub(super) entrypoint: String,
            pub(super) arguments: Vec<String>,
            pub(super) tty: bool,
            pub(super) privileged: bool,
            pub(super) user: String,
        }
        impl Record for CaptureStartExecProcessConfig {
            fn validate(&self) -> Result<()> {
                if self.entrypoint == "/usr/bin/env"
            && self.arguments.len() >= 9
            && self.arguments.len() <= 9
            && self.arguments.first().is_some_and(|item| item == "-i")
            && self.arguments.get(1).is_some_and(|item| item == "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
            && self.arguments.get(2).is_some_and(|item| item == "HOME=/root")
            && self.arguments.get(3).is_some_and(|item| item == "LANG=C")
            && self.arguments.get(4).is_some_and(|item| item == "PGHOST=/var/run/postgresql")
            && self.arguments.get(5).is_some_and(|item| item == "PGPORT=5432")
            && self.arguments.get(6).is_some_and(|item| item == "PGUSER=learning_admin")
            && self.arguments.get(7).is_some_and(|item| item.len() <= 74 && pattern(item, "^PGDATABASE=learning_backup_c4_task3_[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"))
            && self.arguments.get(8).is_some_and(|item| item == "/opt/knowweave-c4/b1-capture/knowweave-c4-b1-capture")
            && !self.tty
            && !self.privileged
            && self.user == "0:0" { Ok(()) } else { Err(invalid()) }
            }
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct Native {
            pub(super) postmaster_start_ticks: u64,
            pub(super) data_dev: u64,
            pub(super) data_ino: u64,
            pub(super) socket_dev: u64,
            pub(super) socket_ino: u64,
        }
        impl Record for Native {
            fn validate(&self) -> Result<()> {
                if self.postmaster_start_ticks >= 1
                    && self.data_dev >= 1
                    && self.data_ino >= 1
                    && self.socket_dev >= 1
                    && self.socket_ino >= 1
                {
                    Ok(())
                } else {
                    Err(invalid())
                }
            }
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct SourceStart {
            pub(super) schema_version: u64,
            pub(super) job_id: String,
            pub(super) launch_id: String,
            pub(super) catalog_sha256: String,
            pub(super) boot_id: String,
            pub(super) root_cookie: String,
            pub(super) capability: String,
            pub(super) launcher: ProcessRef,
            pub(super) source: ProcessRef,
            pub(super) source_control_fd: u64,
            pub(super) source_socket_cookie: u64,
            pub(super) job_lock_dev: u64,
            pub(super) job_lock_ino: u64,
            pub(super) started_monotonic_ns: u64,
        }
        impl Record for SourceStart {
            fn validate(&self) -> Result<()> {
                if self.schema_version == 1
                    && self.job_id.len() <= 36
                    && pattern(
                        &self.job_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && self.launch_id.len() <= 36
                    && pattern(
                        &self.launch_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && self.catalog_sha256.len() <= 64
                    && pattern(&self.catalog_sha256, "^[0-9a-f]{64}$")
                    && self.boot_id.len() <= 36
                    && pattern(
                        &self.boot_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$",
                    )
                    && self.root_cookie.len() <= 64
                    && pattern(&self.root_cookie, "^[0-9a-f]{64}$")
                    && self.capability == "b1_source_process_start_v1"
                    && self.launcher.validate().is_ok()
                    && self.source.validate().is_ok()
                    && self.source_control_fd == 3
                    && self.source_socket_cookie >= 1
                    && self.job_lock_dev >= 1
                    && self.job_lock_ino >= 1
                    && self.started_monotonic_ns >= 1
                {
                    Ok(())
                } else {
                    Err(invalid())
                }
            }
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct ExecutionCatalog {
            pub(super) schema_version: u64,
            pub(super) capability: String,
            pub(super) profile: String,
            pub(super) launch_id: String,
            pub(super) installed_source: InstalledSource,
            pub(super) peer_endpoint: PeerEndpoint,
            pub(super) capture_recipe_sha256: String,
            pub(super) images: Vec<RoleImage>,
            pub(super) installed_runtime: InstalledRuntime,
            pub(super) capture_exec_recipe: CaptureExecRecipe,
        }
        impl Record for ExecutionCatalog {
            fn validate(&self) -> Result<()> {
                if self.schema_version == 1
                    && self.capability == "b1_source_peer_execution_v1"
                    && self.profile == "task9-b1-limited-v1"
                    && self.launch_id.len() <= 36
                    && pattern(
                        &self.launch_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && self.installed_source.validate().is_ok()
                    && self.peer_endpoint.validate().is_ok()
                    && self.capture_recipe_sha256.len() <= 64
                    && pattern(&self.capture_recipe_sha256, "^[0-9a-f]{64}$")
                    && self.images.len() >= 3
                    && self.images.len() <= 3
                    && self.images.iter().all(|item| item.validate().is_ok())
                    && self.installed_runtime.validate().is_ok()
                    && self.capture_exec_recipe.validate().is_ok()
                    && self
                        .images
                        .iter()
                        .map(|i| i.role.as_str())
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        == 3
                    && self.capture_exec_recipe.arguments.get(7)
                        == Some(&format!("PGDATABASE={}", self.installed_source.database))
                {
                    Ok(())
                } else {
                    Err(invalid())
                }
            }
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct RoleImage {
            pub(super) role: String,
            pub(super) executable: CodeFile,
            pub(super) launch_manifest: RecordRef,
            pub(super) libraries: Vec<CodeFile>,
            pub(super) code_commit: String,
            pub(super) build_input_sha256: String,
        }
        impl Record for RoleImage {
            fn validate(&self) -> Result<()> {
                if ["PublicationSource", "Capture", "RootLauncher"].contains(&self.role.as_str())
                    && self.executable.validate().is_ok()
                    && self.launch_manifest.validate().is_ok()
                    && self.libraries.len() <= 32
                    && self.libraries.iter().all(|item| item.validate().is_ok())
                    && self.code_commit.len() <= 40
                    && pattern(&self.code_commit, "^[0-9a-f]{40}$")
                    && self.build_input_sha256.len() <= 64
                    && pattern(&self.build_input_sha256, "^[0-9a-f]{64}$")
                    && match self.role.as_str() {
                        "PublicationSource" => {
                            self.executable.path == "/opt/knowweave-c4/bin/knowweave-c4-b1-source"
                                && self.launch_manifest.record == "launch-manifest.json"
                        }
                        "Capture" => {
                            self.executable.path
                                == "/opt/knowweave-c4/b1-capture/knowweave-c4-b1-capture"
                                && self.launch_manifest.record == "capture-launch-manifest.json"
                        }
                        "RootLauncher" => {
                            self.executable.path
                                == "/opt/knowweave-c4/bin/knowweave-c4-b1-source-launcher"
                                && self.launch_manifest.record == "launcher-launch-manifest.json"
                        }
                        _ => false,
                    }
                    && self.libraries.windows(2).all(|w| w[0].path < w[1].path)
                {
                    Ok(())
                } else {
                    Err(invalid())
                }
            }
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct PeerEndpoint {
            pub(super) capture_directory: String,
            pub(super) source_directory: String,
            pub(super) port: u64,
            pub(super) socket_leaf: String,
            pub(super) directory_dev: u64,
            pub(super) directory_ino: u64,
            pub(super) socket_dev: u64,
            pub(super) socket_ino: u64,
            pub(super) admin_role: String,
            pub(super) broker_role: String,
            pub(super) runtime_role: String,
            pub(super) original_login: String,
        }
        impl Record for PeerEndpoint {
            fn validate(&self) -> Result<()> {
                if self.capture_directory == "/var/run/postgresql"
                    && self.source_directory == "/run/knowweave-c4-source-pg"
                    && self.port == 5432
                    && self.socket_leaf == ".s.PGSQL.5432"
                    && self.directory_dev >= 1
                    && self.directory_ino >= 1
                    && self.socket_dev >= 1
                    && self.socket_ino >= 1
                    && self.admin_role == "learning_admin"
                    && self.broker_role == "learning_snapshot_claim_broker"
                    && self.runtime_role == "learning_runtime"
                    && self.original_login == "peer:root"
                {
                    Ok(())
                } else {
                    Err(invalid())
                }
            }
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct InstalledSource {
            pub(super) deployment_id: String,
            pub(super) source_group_id: String,
            pub(super) application_commit: String,
            pub(super) application_build_sha256: String,
            pub(super) source_auth_public_key_sha256: String,
            pub(super) registry_authority_sha256: String,
            pub(super) source_group_record_sha256: String,
            pub(super) source_control_binding_sha256: String,
            pub(super) database: String,
            pub(super) database_oid: u64,
            pub(super) system_identifier: String,
        }
        impl Record for InstalledSource {
            fn validate(&self) -> Result<()> {
                if self.deployment_id.len() <= 36
                    && pattern(
                        &self.deployment_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && self.source_group_id.len() <= 36
                    && pattern(
                        &self.source_group_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && self.application_commit.len() <= 40
                    && pattern(&self.application_commit, "^[0-9a-f]{40}$")
                    && self.application_build_sha256.len() <= 64
                    && pattern(&self.application_build_sha256, "^[0-9a-f]{64}$")
                    && self.source_auth_public_key_sha256.len() <= 64
                    && pattern(&self.source_auth_public_key_sha256, "^[0-9a-f]{64}$")
                    && self.registry_authority_sha256.len() <= 64
                    && pattern(&self.registry_authority_sha256, "^[0-9a-f]{64}$")
                    && self.source_group_record_sha256.len() <= 64
                    && pattern(&self.source_group_record_sha256, "^[0-9a-f]{64}$")
                    && self.source_control_binding_sha256.len() <= 64
                    && pattern(&self.source_control_binding_sha256, "^[0-9a-f]{64}$")
                    && self.database.len() <= 63
                    && pattern(
                        &self.database,
                        "^learning_backup_c4_task3_[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && self.database_oid >= 1
                    && self.database_oid <= 4294967295
                    && self.system_identifier.len() <= 20
                    && pattern(&self.system_identifier, "^[1-9][0-9]{0,19}$")
                {
                    Ok(())
                } else {
                    Err(invalid())
                }
            }
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct ProcessRef {
            pub(super) host_pid: u64,
            pub(super) namespace_pid: u64,
            pub(super) start_ticks: u64,
            pub(super) pid_namespace: String,
            pub(super) mount_namespace: String,
            pub(super) exe_sha256: String,
            pub(super) exe_dev: u64,
            pub(super) exe_ino: u64,
            pub(super) real_uid: u64,
            pub(super) effective_uid: u64,
            pub(super) saved_uid: u64,
            pub(super) real_gid: u64,
            pub(super) effective_gid: u64,
            pub(super) saved_gid: u64,
        }
        impl Record for ProcessRef {
            fn validate(&self) -> Result<()> {
                if self.host_pid >= 1
                    && self.host_pid <= 4294967295
                    && self.namespace_pid >= 1
                    && self.namespace_pid <= 4294967295
                    && self.start_ticks >= 1
                    && self.pid_namespace.len() <= 64
                    && pattern(&self.pid_namespace, "^pid:\\[[1-9][0-9]*\\]$")
                    && self.mount_namespace.len() <= 64
                    && pattern(&self.mount_namespace, "^mnt:\\[[1-9][0-9]*\\]$")
                    && self.exe_sha256.len() <= 64
                    && pattern(&self.exe_sha256, "^[0-9a-f]{64}$")
                    && self.exe_dev >= 1
                    && self.exe_ino >= 1
                    && self.real_uid == 0
                    && self.effective_uid == 0
                    && self.saved_uid == 0
                    && self.real_gid == 0
                    && self.effective_gid == 0
                    && self.saved_gid == 0
                {
                    Ok(())
                } else {
                    Err(invalid())
                }
            }
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct CodeFile {
            pub(super) path: String,
            pub(super) stamp: FileStamp,
            pub(super) sha256: String,
        }
        impl Record for CodeFile {
            fn validate(&self) -> Result<()> {
                if self.path.len() <= 1024
                    && pattern(&self.path, "^/(?!/)(?!.*(?:^|/)\\.\\.?(?:/|$))[^\\x00]+$")
                    && self.stamp.validate().is_ok()
                    && self.sha256.len() <= 64
                    && pattern(&self.sha256, "^[0-9a-f]{64}$")
                {
                    Ok(())
                } else {
                    Err(invalid())
                }
            }
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct FileStamp {
            pub(super) dev: u64,
            pub(super) ino: u64,
            pub(super) uid: u64,
            pub(super) gid: u64,
            pub(super) mode: u64,
            pub(super) nlink: u64,
            pub(super) bytes: u64,
            pub(super) mtime_ns: u64,
            pub(super) ctime_ns: u64,
        }
        impl Record for FileStamp {
            fn validate(&self) -> Result<()> {
                if self.dev >= 1
                    && self.ino >= 1
                    && self.uid == 0
                    && self.gid <= 4294967295
                    && self.mode <= 65535
                    && self.nlink == 1
                    && self.bytes >= 1
                    && self.bytes <= 134217728
                {
                    Ok(())
                } else {
                    Err(invalid())
                }
            }
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct RecordRef {
            pub(super) record: String,
            pub(super) bytes: u64,
            pub(super) sha256: String,
        }
        impl Record for RecordRef {
            fn validate(&self) -> Result<()> {
                if [
                    "launch-manifest.json",
                    "capture-launch-manifest.json",
                    "launcher-launch-manifest.json",
                    "source-peer-execution.json",
                    "source-start.json",
                    "capture-start.json",
                    "generation-binding.json",
                    "captured-data.json",
                    "capture-settlement.json",
                ]
                .contains(&self.record.as_str())
                    && self.bytes >= 1
                    && self.bytes <= 16384
                    && self.sha256.len() <= 64
                    && pattern(&self.sha256, "^[0-9a-f]{64}$")
                {
                    Ok(())
                } else {
                    Err(invalid())
                }
            }
        }

        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct LaunchManifest {
            pub(super) format_version: u32,
            pub(super) capability: String,
            pub(super) profile: String,
            pub(super) launch_id: String,
            pub(super) role: String,
            pub(super) code_commit: String,
            pub(super) build_input_sha256: String,
            pub(super) binary_sha256: String,
            pub(super) launcher_sha256: String,
            pub(super) policy_sha256: String,
            pub(super) anchor_sha256: String,
            pub(super) source_auth_public_key_sha256: String,
            pub(super) destination_verifier_public_key_sha256: String,
            pub(super) transport_public_key_sha256: String,
        }
        impl Record for LaunchManifest {
            fn validate(&self) -> Result<()> {
                if self.format_version != 1
                    || self.capability != "task9_b1_launch_manifest_v1"
                    || self.profile != "task9-b1-limited-v1"
                    || self.role != "source"
                    || !uuid(&self.launch_id, true)
                    || !hex(&self.code_commit, 40)
                    || [
                        &self.build_input_sha256,
                        &self.binary_sha256,
                        &self.launcher_sha256,
                        &self.policy_sha256,
                        &self.anchor_sha256,
                        &self.source_auth_public_key_sha256,
                        &self.destination_verifier_public_key_sha256,
                        &self.transport_public_key_sha256,
                    ]
                    .iter()
                    .any(|v| !hex(v, 64))
                {
                    return Err(invalid());
                }
                Ok(())
            }
        }
    }

    pub(super) fn require_actual_root() -> Result<()> {
        let (mut r, mut e, mut s) = (u32::MAX, u32::MAX, u32::MAX);
        let (mut gr, mut ge, mut gs) = (u32::MAX, u32::MAX, u32::MAX);
        if unsafe { libc::getresuid(&mut r, &mut e, &mut s) } != 0
            || unsafe { libc::getresgid(&mut gr, &mut ge, &mut gs) } != 0
        {
            return Err(invalid());
        }
        if (r, e, s, gr, ge, gs) != (0, 0, 0, 0, 0, 0) {
            return Err(invalid());
        }
        Ok(())
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(super) struct Stamp {
        pub(super) dev: u64,
        pub(super) ino: u64,
        pub(super) uid: u32,
        pub(super) gid: u32,
        pub(super) mode: u32,
        pub(super) links: u64,
        pub(super) bytes: u64,
        pub(super) mtime_ns: u64,
        pub(super) ctime_ns: u64,
    }
    impl Stamp {
        pub(super) fn observe(file: &File, scan: &SharedJournalBudget) -> Result<Self> {
            scan_charge(scan, 0, 1).map_err(|_| invalid())?;
            let m = file.metadata().map_err(|_| invalid())?;
            let nanos = |s: i64, n: i64| -> Result<u64> {
                u64::try_from(s)
                    .ok()
                    .and_then(|s| s.checked_mul(1_000_000_000))
                    .and_then(|s| {
                        u64::try_from(n)
                            .ok()
                            .filter(|v| *v < 1_000_000_000)
                            .and_then(|n| s.checked_add(n))
                    })
                    .ok_or(invalid())
            };
            Ok(Self {
                dev: m.dev(),
                ino: m.ino(),
                uid: m.uid(),
                gid: m.gid(),
                mode: m.mode(),
                links: m.nlink(),
                bytes: m.len(),
                mtime_ns: nanos(m.mtime(), m.mtime_nsec())?,
                ctime_ns: nanos(m.ctime(), m.ctime_nsec())?,
            })
        }
        fn same_declared(&self, d: &job::FileStamp) -> bool {
            self.dev == d.dev
                && self.ino == d.ino
                && u64::from(self.uid) == d.uid
                && u64::from(self.gid) == d.gid
                && u64::from(self.mode) == d.mode
                && self.links == d.nlink
                && self.bytes == d.bytes
                && self.mtime_ns == d.mtime_ns
                && self.ctime_ns == d.ctime_ns
        }
        fn root_file(&self, maximum: u64) -> bool {
            self.dev > 0
                && self.ino > 0
                && self.uid == 0
                && self.links == 1
                && self.mode & libc::S_IFMT == libc::S_IFREG
                && self.mode & 0o022 == 0
                && self.bytes > 0
                && self.bytes <= maximum
        }
        fn same_directory_identity(&self, other: &Self) -> bool {
            self.dev == other.dev
                && self.ino == other.ino
                && self.uid == other.uid
                && self.gid == other.gid
                && self.mode == other.mode
                && self.mode & libc::S_IFMT == libc::S_IFDIR
                && self.uid == 0
                && self.mode & 0o022 == 0
        }
    }

    pub(super) struct HeldCode {
        held: HeldNamed,
        digest: [u8; 32],
    }
    impl HeldCode {
        fn open(
            declared: &job::CodeFile,
            manager: bool,
            scan: &SharedJournalBudget,
        ) -> Result<Self> {
            let mut held = HeldNamed::open(&declared.path, scan)?;
            if !held.stamp.same_declared(&declared.stamp)
                || manager && held.stamp.mode & 0o7777 != 0o500
            {
                return Err(invalid());
            }
            let (bytes, digest) = held.full_read_and_hash(128 * 1024 * 1024, scan)?;
            if !bytes.starts_with(b"\x7fELF") || encode_hex(&digest) != declared.sha256 {
                return Err(invalid());
            }
            Ok(Self { held, digest })
        }
        pub(super) fn revalidate(&self, scan: &SharedJournalBudget) -> Result<()> {
            self.held.revalidate(scan)
        }
        pub(super) fn matches_actual_exe(
            &self,
            pid: u32,
            scan: &SharedJournalBudget,
        ) -> Result<()> {
            if pid == 0 {
                return Err(invalid());
            }
            let view = ProcView::open(scan)?;
            let actual = view.open_checked_magic(&format!("{pid}/exe"), scan)?;
            // This actual current-task/parent ELF is joined to the fixed held file;
            // the proc magic link never substitutes a named-code declaration.
            if Stamp::observe(&actual, scan)? != self.held.stamp {
                return Err(invalid());
            }
            Ok(())
        }
    }
    pub(super) fn require_cloexec(fd: RawFd) -> Result<()> {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if flags < 0 || flags & libc::FD_CLOEXEC == 0 {
            return Err(invalid());
        }
        Ok(())
    }
    pub(super) fn mark_cloexec_nonblocking(fd: RawFd) -> Result<()> {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
            return Err(invalid());
        }
        let status = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if status < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, status | libc::O_NONBLOCK) } < 0 {
            return Err(invalid());
        }
        require_cloexec(fd)
    }
    pub(super) fn poll_fd(fd: RawFd, events: i16, deadline: Instant) -> Result<()> {
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or(invalid())?;
            let milliseconds = remaining
                .as_millis()
                .saturating_add(1)
                .min(i32::MAX as u128) as i32;
            let mut p = libc::pollfd {
                fd,
                events,
                revents: 0,
            };
            let code = unsafe { libc::poll(&mut p, 1, milliseconds) };
            if code < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
            {
                continue;
            }
            if code < 0 {
                return Err(invalid());
            }
            if code == 0 {
                return Err(invalid());
            }
            if p.revents & libc::POLLNVAL != 0 {
                return Err(invalid());
            }
            if p.revents & events != 0 {
                return Ok(());
            }
            if p.revents & (libc::POLLERR | libc::POLLHUP) != 0 {
                return Err(invalid());
            }
            return Err(invalid());
        }
    }
    pub(super) struct ParentPid {
        fd: OwnedFd,
        pid: u32,
        start: u64,
    }
    impl ParentPid {
        pub(super) fn open(pid: u32, start: u64, scan: &SharedJournalBudget) -> Result<Self> {
            if pid == 0 || pid > i32::MAX as u32 || start == 0 {
                return Err(invalid());
            }
            scan_charge(scan, 0, 1).map_err(|_| invalid())?;
            let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
            if raw < 0 {
                return Err(invalid());
            }
            let fd = unsafe { OwnedFd::from_raw_fd(i32::try_from(raw).map_err(|_| invalid())?) };
            let result = Self { fd, pid, start };
            result.revalidate(scan)?;
            Ok(result)
        }
        pub(super) fn revalidate(&self, scan: &SharedJournalBudget) -> Result<()> {
            scan_charge(scan, 0, 2).map_err(|_| invalid())?;
            require_cloexec(self.fd.as_raw_fd())?;
            let mut p = libc::pollfd {
                fd: self.fd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            if unsafe { libc::poll(&mut p, 1, 0) } != 0
                || actual_start(self.pid, scan)? != self.start
            {
                return Err(invalid());
            }
            Ok(())
        }
    }
    pub(super) fn parse_start(stat: &[u8], pid: u32) -> Result<u64> {
        let text = std::str::from_utf8(stat).map_err(|_| invalid())?;
        let (head, tail) = text.rsplit_once(')').ok_or(invalid())?;
        if !head.starts_with(&format!("{pid} (")) {
            return Err(invalid());
        }
        let value = tail.split_ascii_whitespace().nth(19).ok_or(invalid())?;
        let ticks = value.parse::<u64>().map_err(|_| invalid())?;
        if ticks == 0 || ticks.to_string() != value {
            return Err(invalid());
        }
        Ok(ticks)
    }
    pub(super) fn actual_start(pid: u32, scan: &SharedJournalBudget) -> Result<u64> {
        let bytes = ProcView::open(scan)?.read_relative(&format!("{pid}/stat"), 4096, scan)?;
        parse_start(&bytes, pid)
    }
    const PROC_MAGIC: libc::c_long = 0x9fa0;
    const NSFS_MAGIC: libc::c_long = 0x6e736673;
    const NS_GET_NSTYPE: libc::c_ulong = 0xb703;
    const RESOLVE_NO_XDEV: u64 = 0x01;
    const RESOLVE_NO_SYMLINKS: u64 = 0x04;
    const RESOLVE_BENEATH: u64 = 0x08;
    #[repr(C)]
    struct OpenHow {
        flags: u64,
        mode: u64,
        resolve: u64,
    }
    #[derive(Clone, Copy, PartialEq, Eq)]
    struct NativeObject {
        dev: u64,
        ino: u64,
        kind: u32,
    }
    fn actual_object(file: &File, scan: &SharedJournalBudget) -> Result<NativeObject> {
        scan_charge(scan, 0, 1).map_err(|_| invalid())?;
        let m = file.metadata().map_err(|_| invalid())?;
        Ok(NativeObject {
            dev: m.dev(),
            ino: m.ino(),
            kind: m.mode() & libc::S_IFMT,
        })
    }
    fn require_filesystem(
        file: &File,
        kind: libc::c_long,
        scan: &SharedJournalBudget,
    ) -> Result<()> {
        scan_charge(scan, 0, 1).map_err(|_| invalid())?;
        let mut fs = std::mem::MaybeUninit::<libc::statfs>::uninit();
        if unsafe { libc::fstatfs(file.as_raw_fd(), fs.as_mut_ptr()) } != 0 {
            return Err(invalid());
        }
        if unsafe { fs.assume_init() }.f_type != kind {
            return Err(invalid());
        }
        Ok(())
    }
    fn mount_id(file: &File, scan: &SharedJournalBudget) -> Result<u64> {
        scan_charge(scan, 0, 1).map_err(|_| invalid())?;
        let mut st = std::mem::MaybeUninit::<libc::statx>::zeroed();
        if unsafe {
            libc::statx(
                file.as_raw_fd(),
                c"".as_ptr(),
                libc::AT_EMPTY_PATH | libc::AT_SYMLINK_NOFOLLOW,
                libc::STATX_MNT_ID,
                st.as_mut_ptr(),
            )
        } != 0
        {
            return Err(invalid());
        }
        let st = unsafe { st.assume_init() };
        if st.stx_mask & libc::STATX_MNT_ID == 0 || st.stx_mnt_id == 0 {
            return Err(invalid());
        }
        Ok(st.stx_mnt_id)
    }
    pub(super) fn require_same_current_pid(
        original: u32,
        scan: &SharedJournalBudget,
    ) -> Result<()> {
        scan_charge(scan, 0, 1).map_err(|_| invalid())?;
        let current = unsafe { libc::getpid() };
        if original == 0 || current <= 0 || current as u32 != original {
            return Err(invalid());
        }
        Ok(())
    }
    /// DATA parser only. Linux pads these decimal columns with spaces.
    pub(super) fn supported_identity_mapping(raw: &[u8]) -> Result<()> {
        if raw.is_empty() || raw.len() > 256 || !raw.is_ascii() {
            return Err(invalid());
        }
        let text = std::str::from_utf8(raw).map_err(|_| invalid())?;
        let text = text.strip_suffix('\n').unwrap_or(text);
        if text.contains(['\n', '\r'])
            || text
                .bytes()
                .any(|b| b != b' ' && b != b'\t' && !b.is_ascii_digit())
        {
            return Err(invalid());
        }
        let fields: Vec<_> = text.split_ascii_whitespace().collect();
        if fields.len() != 3 {
            return Err(invalid());
        }
        let mut values = [0_u64; 3];
        for (index, value) in fields.iter().enumerate() {
            values[index] = value.parse().map_err(|_| invalid())?;
            if values[index].to_string() != *value {
                return Err(invalid());
            }
        }
        if values != [0, 0, u64::from(u32::MAX)] {
            return Err(invalid());
        }
        Ok(())
    }
    pub(super) fn proc_root_mount_descriptor(raw: &[u8], wanted: u64, dev: u64) -> Result<String> {
        if raw.is_empty() || !raw.is_ascii() {
            return Err(invalid());
        }
        let text = std::str::from_utf8(raw).map_err(|_| invalid())?;
        let mut found = None;
        for line in text.lines() {
            let (left, right) = line.split_once(" - ").ok_or(invalid())?;
            let a: Vec<_> = left.split_ascii_whitespace().collect();
            let b: Vec<_> = right.split_ascii_whitespace().collect();
            if a.len() < 6 || b.len() != 3 {
                return Err(invalid());
            }
            let id = a[0].parse::<u64>().map_err(|_| invalid())?;
            if a[4] == "/proc" || id == wanted {
                let (major, minor) = a[2].split_once(':').ok_or(invalid())?;
                let major = major.parse::<u32>().map_err(|_| invalid())?;
                let minor = minor.parse::<u32>().map_err(|_| invalid())?;
                if found.is_some()
                    || id != wanted
                    || a[3] != "/"
                    || a[4] != "/proc"
                    || b[0] != "proc"
                    || major != libc::major(dev)
                    || minor != libc::minor(dev)
                {
                    return Err(invalid());
                }
                found = Some(line.to_owned());
            }
        }
        found.ok_or(invalid())
    }
    /// A held, genuine proc mount with kernel-generated `self` pointing to this
    /// actual current PID. Numeric task/leaf opens refuse every child mount/bind.
    /// This does not identify the global initial user namespace or its custodian.
    pub(super) struct ProcView {
        root: Arc<File>,
        object: NativeObject,
        mount: u64,
        own_pid: u32,
        descriptor: String,
    }
    impl ProcView {
        pub(super) fn open(scan: &SharedJournalBudget) -> Result<Self> {
            scan_charge(scan, 5, 1).map_err(|_| invalid())?;
            let raw = unsafe {
                libc::open(
                    c"/proc".as_ptr(),
                    libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if raw < 0 {
                return Err(invalid());
            }
            let root = share_native(unsafe { File::from_raw_fd(raw) }, scan)?;
            require_filesystem(&root, PROC_MAGIC, scan)?;
            let object = actual_object(&root, scan)?;
            if object.kind != libc::S_IFDIR {
                return Err(invalid());
            }
            let mount = mount_id(&root, scan)?;
            scan_charge(scan, 0, 1).map_err(|_| invalid())?;
            let own_pid = u32::try_from(unsafe { libc::getpid() }).map_err(|_| invalid())?;
            if own_pid == 0 {
                return Err(invalid());
            }
            let mut view = Self {
                root,
                object,
                mount,
                own_pid,
                descriptor: String::new(),
            };
            if view.read_link_relative("self", scan)? != own_pid.to_string() {
                return Err(invalid());
            }
            view.descriptor = proc_root_mount_descriptor(
                &view.read_relative(&format!("{own_pid}/mountinfo"), 1024 * 1024, scan)?,
                mount,
                object.dev,
            )?;
            view.revalidate_root(scan)?;
            Ok(view)
        }
        fn open_relative(
            &self,
            relative: &str,
            flags: i32,
            scan: &SharedJournalBudget,
        ) -> Result<File> {
            if relative.is_empty()
                || relative.starts_with('/')
                || relative
                    .split('/')
                    .any(|p| p.is_empty() || p == "." || p == "..")
            {
                return Err(invalid());
            }
            let path = CString::new(relative).map_err(|_| invalid())?;
            let how = OpenHow {
                flags: (flags | libc::O_CLOEXEC) as u64,
                mode: 0,
                resolve: RESOLVE_BENEATH | RESOLVE_NO_XDEV | RESOLVE_NO_SYMLINKS,
            };
            scan_charge(scan, path.as_bytes().len() as u64, 1).map_err(|_| invalid())?;
            let raw = unsafe {
                libc::syscall(
                    libc::SYS_openat2,
                    self.root.as_raw_fd(),
                    path.as_ptr(),
                    &how,
                    std::mem::size_of::<OpenHow>(),
                )
            };
            if raw < 0 {
                return Err(invalid());
            }
            let file = unsafe { File::from_raw_fd(i32::try_from(raw).map_err(|_| invalid())?) };
            require_filesystem(&file, PROC_MAGIC, scan)?;
            if mount_id(&file, scan)? != self.mount {
                return Err(invalid());
            }
            Ok(file)
        }
        pub(super) fn read_relative(
            &self,
            relative: &str,
            maximum: usize,
            scan: &SharedJournalBudget,
        ) -> Result<Vec<u8>> {
            pay(scan, relative.len() * 8 + 1024, 4)?;
            let file = self.open_relative(relative, libc::O_RDONLY | libc::O_NOFOLLOW, scan)?;
            if actual_object(&file, scan)?.kind != libc::S_IFREG {
                return Err(invalid());
            }
            let cap = maximum.checked_add(1).ok_or_else(invalid)?;
            pay(scan, cap, 1)?;
            let mut raw = Vec::with_capacity(cap);
            loop {
                let want = (cap - raw.len()).min(8192);
                if want == 0 {
                    return Err(invalid());
                }
                let mut b = [0_u8; 8192];
                pay(scan, want, 1)?;
                let n = unsafe { libc::read(file.as_raw_fd(), b.as_mut_ptr().cast(), want) };
                if n == 0 {
                    break;
                }
                if n < 0 {
                    if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(invalid());
                }
                let n = n as usize;
                if raw.len() + n > maximum {
                    return Err(invalid());
                }
                raw.extend_from_slice(&b[..n]);
            }
            pay(scan, raw.len() * 8 + 1024, 1)?;
            Ok(raw)
        }
        fn link(&self, relative: &str, scan: &SharedJournalBudget) -> Result<File> {
            let link = self.open_relative(relative, libc::O_PATH | libc::O_NOFOLLOW, scan)?;
            if actual_object(&link, scan)?.kind != libc::S_IFLNK {
                return Err(invalid());
            }
            Ok(link)
        }
        fn link_value(link: &File, scan: &SharedJournalBudget) -> Result<String> {
            scan_charge(scan, 4097, 1).map_err(|_| invalid())?;
            let mut raw = [0_u8; 4097];
            let len = unsafe {
                libc::readlinkat(
                    link.as_raw_fd(),
                    c"".as_ptr(),
                    raw.as_mut_ptr().cast(),
                    raw.len(),
                )
            };
            if len <= 0 || len as usize >= raw.len() {
                return Err(invalid());
            }
            String::from_utf8(raw[..len as usize].to_vec()).map_err(|_| invalid())
        }
        fn read_link_relative(&self, relative: &str, scan: &SharedJournalBudget) -> Result<String> {
            Self::link_value(&self.link(relative, scan)?, scan)
        }
        fn relative_path(&self, path: &str, scan: &SharedJournalBudget) -> Result<String> {
            let path = path.strip_prefix("/proc/").ok_or(invalid())?;
            if let Some(tail) = path.strip_prefix("self/") {
                require_same_current_pid(self.own_pid, scan)?;
                Ok(format!("{}/{tail}", self.own_pid))
            } else {
                Ok(path.to_owned())
            }
        }
        fn require_single_task(&self, scan: &SharedJournalBudget) -> Result<()> {
            let directory = self.open_relative(
                &format!("{}/task", self.own_pid),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW,
                scan,
            )?;
            let mut count = 0_usize;
            let mut buffer = [0_u8; 4096];
            loop {
                scan_charge(scan, buffer.len() as u64, 1).map_err(|_| invalid())?;
                let size = unsafe {
                    libc::syscall(
                        libc::SYS_getdents64,
                        directory.as_raw_fd(),
                        buffer.as_mut_ptr(),
                        buffer.len(),
                    )
                };
                if size < 0 {
                    return Err(invalid());
                }
                if size == 0 {
                    break;
                }
                let size = usize::try_from(size).map_err(|_| invalid())?;
                if size > buffer.len() {
                    return Err(invalid());
                }
                let mut offset = 0;
                while offset < size {
                    if size - offset < 20 {
                        return Err(invalid());
                    }
                    let length = usize::from(u16::from_ne_bytes([
                        buffer[offset + 16],
                        buffer[offset + 17],
                    ]));
                    if length < 20 || length > size - offset {
                        return Err(invalid());
                    }
                    let raw = &buffer[offset + 19..offset + length];
                    let end = raw.iter().position(|byte| *byte == 0).ok_or(invalid())?;
                    let name = std::str::from_utf8(&raw[..end]).map_err(|_| invalid())?;
                    scan_charge(scan, name.len() as u64, 1).map_err(|_| invalid())?;
                    if name != "." && name != ".." {
                        if name != self.own_pid.to_string() || count != 0 {
                            return Err(invalid());
                        }
                        count += 1;
                    }
                    offset += length;
                }
            }
            if count != 1 {
                return Err(invalid());
            }
            self.revalidate_root(scan)
        }
        fn open_checked_magic(&self, relative: &str, scan: &SharedJournalBudget) -> Result<File> {
            let (parent, name) = relative.rsplit_once('/').ok_or(invalid())?;
            let parent = self.open_relative(
                parent,
                libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW,
                scan,
            )?;
            let before = self.link(relative, scan)?;
            let identity = actual_object(&before, scan)?;
            let name = CString::new(name).map_err(|_| invalid())?;
            scan_charge(scan, name.as_bytes().len() as u64, 1).map_err(|_| invalid())?;
            // Only this final, previously held genuine proc magic link is followed.
            // The parent was opened NO_XDEV; the link and mount are checked again.
            let raw = unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_CLOEXEC,
                )
            };
            if raw < 0 {
                return Err(invalid());
            }
            let target = unsafe { File::from_raw_fd(raw) };
            // Descent has finished. The original genuine proc link and target are
            // still held against the same original proc root while the current
            // named link/bytes are checked; this parent reader adds no authority.
            drop(parent);
            if actual_object(&self.link(relative, scan)?, scan)? != identity
                || Self::link_value(&before, scan)? != self.read_link_relative(relative, scan)?
            {
                return Err(invalid());
            }
            // The original target remains held; the checked link and its parent
            // have completed this read duty before proc-root revalidation opens
            // its own independent named/link probes.
            drop(before);
            self.revalidate_root(scan)?;
            Ok(target)
        }
        fn revalidate_root(&self, scan: &SharedJournalBudget) -> Result<()> {
            require_same_current_pid(self.own_pid, scan)?;
            scan_charge(scan, 5, 1).map_err(|_| invalid())?;
            let raw = unsafe {
                libc::open(
                    c"/proc".as_ptr(),
                    libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if raw < 0 {
                return Err(invalid());
            }
            let named = unsafe { File::from_raw_fd(raw) };
            require_filesystem(&named, PROC_MAGIC, scan)?;
            require_filesystem(&self.root, PROC_MAGIC, scan)?;
            if actual_object(&named, scan)? != self.object
                || actual_object(&self.root, scan)? != self.object
                || mount_id(&named, scan)? != self.mount
                || mount_id(&self.root, scan)? != self.mount
                || self.read_link_relative("self", scan)? != self.own_pid.to_string()
            {
                return Err(invalid());
            }
            let descriptor = proc_root_mount_descriptor(
                &self.read_relative(&format!("{}/mountinfo", self.own_pid), 1024 * 1024, scan)?,
                self.mount,
                self.object.dev,
            )?;
            if descriptor != self.descriptor {
                return Err(invalid());
            }
            Ok(())
        }
        pub(super) fn user_namespace(
            &self,
            pid: u32,
            scan: &SharedJournalBudget,
        ) -> Result<NamespaceObject> {
            if pid == 0 {
                return Err(invalid());
            }
            let relative = format!("{pid}/ns/user");
            let link = self.link(&relative, scan)?;
            let target = Self::link_value(&link, scan)?;
            let value = target
                .strip_prefix("user:[")
                .and_then(|s| s.strip_suffix(']'))
                .ok_or(invalid())?;
            let expected = value.parse::<u64>().map_err(|_| invalid())?;
            if expected == 0 || expected.to_string() != value {
                return Err(invalid());
            }
            drop(link);
            let file = self.open_checked_magic(&relative, scan)?;
            require_user_namespace_fd(&file, scan)?;
            let object = actual_object(&file, scan)?;
            if object.ino != expected {
                return Err(invalid());
            }
            Ok(NamespaceObject {
                file: share_native(file, scan)?,
                object,
            })
        }
    }
    pub(super) fn require_user_namespace_fd(file: &File, scan: &SharedJournalBudget) -> Result<()> {
        require_filesystem(file, NSFS_MAGIC, scan)?;
        scan_charge(scan, 0, 2).map_err(|_| invalid())?;
        require_cloexec(file.as_raw_fd())?;
        if unsafe { libc::ioctl(file.as_raw_fd(), NS_GET_NSTYPE) } != libc::CLONE_NEWUSER {
            return Err(invalid());
        }
        Ok(())
    }
    pub(super) struct NamespaceObject {
        file: Arc<File>,
        object: NativeObject,
    }
    impl NamespaceObject {
        fn revalidate(&self, scan: &SharedJournalBudget) -> Result<()> {
            require_user_namespace_fd(&self.file, scan)?;
            if actual_object(&self.file, scan)? != self.object {
                return Err(invalid());
            }
            Ok(())
        }
        pub(super) fn same_object(&self, other: &Self) -> bool {
            self.object == other.object
        }
    }
    // Fixed boot-ID compatibility only. This DATA parser does not attest a native
    // view; ProcSysView independently opens and joins every returned mount object.
    struct NativeSysMount {
        mount: u64,
        descriptor: String,
    }
    enum ProcSysShapeKind {
        Direct,
        NativeReadOnly(NativeSysMount),
    }
    pub(super) struct ProcSysShape {
        kind: ProcSysShapeKind,
    }
    impl ProcSysShape {
        pub(super) fn direct_data(&self) -> bool {
            matches!(&self.kind, ProcSysShapeKind::Direct)
        }
        pub(super) fn native_ro_mount_data(&self) -> Option<u64> {
            match &self.kind {
                ProcSysShapeKind::Direct => None,
                ProcSysShapeKind::NativeReadOnly(row) => Some(row.mount),
            }
        }
    }
    pub(super) fn parse_proc_sys_shape(
        raw: &[u8],
        proc_mount: u64,
        proc_dev: u64,
    ) -> Result<ProcSysShape> {
        if raw.is_empty() || raw.len() > 1024 * 1024 || !raw.is_ascii() || proc_mount == 0 {
            return Err(invalid());
        }
        let text = std::str::from_utf8(raw).map_err(|_| invalid())?;
        if !text.ends_with('\n') || text.contains('\r') {
            return Err(invalid());
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut selected = None;
        let mut proc_seen = 0;
        for line in text.lines() {
            let (left, right) = line.split_once(" - ").ok_or(invalid())?;
            let a: Vec<_> = left.split_ascii_whitespace().collect();
            let b: Vec<_> = right.split_ascii_whitespace().collect();
            if a.len() < 6 || b.len() != 3 {
                return Err(invalid());
            }
            let decimal = |s: &str| -> Result<u64> {
                let value = s.parse::<u64>().map_err(|_| invalid())?;
                if value == 0 || value.to_string() != s {
                    return Err(invalid());
                }
                Ok(value)
            };
            let id = decimal(a[0])?;
            let parent = decimal(a[1])?;
            if !ids.insert(id) {
                return Err(invalid());
            }
            let target = a[4];
            if target == "/proc" {
                proc_seen += 1;
                let (major, minor) = a[2].split_once(':').ok_or(invalid())?;
                let major = major.parse::<u32>().map_err(|_| invalid())?;
                let minor = minor.parse::<u32>().map_err(|_| invalid())?;
                if id != proc_mount
                    || a[3] != "/"
                    || b[0] != "proc"
                    || major != libc::major(proc_dev)
                    || minor != libc::minor(proc_dev)
                {
                    return Err(invalid());
                }
            }
            if target.starts_with("/proc/") && target.contains('\\') {
                return Err(invalid());
            }
            if target.starts_with("/proc/sys/") {
                return Err(invalid());
            }
            if target == "/proc/sys" {
                let (major, minor) = a[2].split_once(':').ok_or(invalid())?;
                let major = major.parse::<u32>().map_err(|_| invalid())?;
                let minor = minor.parse::<u32>().map_err(|_| invalid())?;
                let options: Vec<_> = a[5].split(',').collect();
                let flags_unique = options
                    .iter()
                    .copied()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    == options.len();
                if selected.is_some()
                    || id == proc_mount
                    || parent != proc_mount
                    || a[3] != "/sys"
                    || b[0] != "proc"
                    || major != libc::major(proc_dev)
                    || minor != libc::minor(proc_dev)
                    || !flags_unique
                    || options.contains(&"rw")
                    || !["ro", "nosuid", "nodev", "noexec"]
                        .iter()
                        .all(|v| options.contains(v))
                {
                    return Err(invalid());
                }
                // Superblock options b[2] may be rw for a native RO bind; only the
                // actual selected mount's per-mount options determine this gate.
                selected = Some(NativeSysMount {
                    mount: id,
                    descriptor: line.to_owned(),
                });
            }
        }
        if proc_seen != 1 {
            return Err(invalid());
        }
        Ok(ProcSysShape {
            kind: selected.map_or(ProcSysShapeKind::Direct, ProcSysShapeKind::NativeReadOnly),
        })
    }
    enum ProcSysKind {
        Direct,
        NativeReadOnly {
            root: File,
            object: NativeObject,
            row: NativeSysMount,
        },
    }
    struct ProcSysView {
        kind: ProcSysKind,
    }
    impl ProcSysView {
        fn mountinfo(proc: &ProcView, scan: &SharedJournalBudget) -> Result<Vec<u8>> {
            proc.read_relative(&format!("{}/mountinfo", proc.own_pid), 1024 * 1024, scan)
        }
        fn open_sys_root(proc: &ProcView, scan: &SharedJournalBudget) -> Result<File> {
            let how = OpenHow {
                flags: (libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                    as u64,
                mode: 0,
                resolve: RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS,
            };
            scan_charge(scan, 3, 1).map_err(|_| invalid())?;
            // The one intentionally permitted crossing is the literal component
            // sys. No generic proc opener loses NO_XDEV, and no caller path exists.
            let raw = unsafe {
                libc::syscall(
                    libc::SYS_openat2,
                    proc.root.as_raw_fd(),
                    c"sys".as_ptr(),
                    &how,
                    std::mem::size_of::<OpenHow>(),
                )
            };
            if raw < 0 {
                return Err(invalid());
            }
            let root = unsafe { File::from_raw_fd(i32::try_from(raw).map_err(|_| invalid())?) };
            require_filesystem(&root, PROC_MAGIC, scan)?;
            let object = actual_object(&root, scan)?;
            if object.kind != libc::S_IFDIR || object.dev != proc.object.dev {
                return Err(invalid());
            }
            Ok(root)
        }
        fn require_ro(root: &File, scan: &SharedJournalBudget) -> Result<()> {
            scan_charge(scan, 0, 1).map_err(|_| invalid())?;
            let mut st = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
            if unsafe { libc::fstatvfs(root.as_raw_fd(), st.as_mut_ptr()) } != 0 {
                return Err(invalid());
            }
            let st = unsafe { st.assume_init() };
            if st.f_flag & libc::ST_RDONLY == 0 || st.f_flag & libc::ST_NOSUID == 0 {
                return Err(invalid());
            }
            Ok(())
        }
        fn observe(proc: &ProcView, scan: &SharedJournalBudget) -> Result<Self> {
            proc.revalidate_root(scan)?;
            let shape =
                parse_proc_sys_shape(&Self::mountinfo(proc, scan)?, proc.mount, proc.object.dev)?;
            let kind = match shape.kind {
                ProcSysShapeKind::Direct => ProcSysKind::Direct,
                ProcSysShapeKind::NativeReadOnly(row) => {
                    let root = Self::open_sys_root(proc, scan)?;
                    let object = actual_object(&root, scan)?;
                    if mount_id(&root, scan)? != row.mount {
                        return Err(invalid());
                    }
                    Self::require_ro(&root, scan)?;
                    ProcSysKind::NativeReadOnly { root, object, row }
                }
            };
            let view = Self { kind };
            view.revalidate(proc, scan)?;
            Ok(view)
        }
        fn revalidate(&self, proc: &ProcView, scan: &SharedJournalBudget) -> Result<()> {
            proc.revalidate_root(scan)?;
            let now =
                parse_proc_sys_shape(&Self::mountinfo(proc, scan)?, proc.mount, proc.object.dev)?;
            match (&self.kind, now.kind) {
                (ProcSysKind::Direct, ProcSysShapeKind::Direct) => {}
                (
                    ProcSysKind::NativeReadOnly { root, object, row },
                    ProcSysShapeKind::NativeReadOnly(now),
                ) => {
                    if now.mount != row.mount || now.descriptor != row.descriptor {
                        return Err(invalid());
                    }
                    let named = Self::open_sys_root(proc, scan)?;
                    require_filesystem(root, PROC_MAGIC, scan)?;
                    if actual_object(root, scan)? != *object
                        || actual_object(&named, scan)? != *object
                        || mount_id(root, scan)? != row.mount
                        || mount_id(&named, scan)? != row.mount
                    {
                        return Err(invalid());
                    }
                    Self::require_ro(root, scan)?;
                    Self::require_ro(&named, scan)?;
                }
                _ => return Err(invalid()),
            }
            proc.revalidate_root(scan)
        }
        fn boot_id(&self, proc: &ProcView, scan: &SharedJournalBudget) -> Result<String> {
            self.revalidate(proc, scan)?;
            let raw = match &self.kind {
                ProcSysKind::Direct => proc.read_relative("sys/kernel/random/boot_id", 37, scan)?,
                ProcSysKind::NativeReadOnly { root, object, row } => {
                    let how = OpenHow {
                        flags: (libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC) as u64,
                        mode: 0,
                        resolve: RESOLVE_BENEATH | RESOLVE_NO_XDEV | RESOLVE_NO_SYMLINKS,
                    };
                    scan_charge(scan, 21, 1).map_err(|_| invalid())?;
                    let raw = unsafe {
                        libc::syscall(
                            libc::SYS_openat2,
                            root.as_raw_fd(),
                            c"kernel/random/boot_id".as_ptr(),
                            &how,
                            std::mem::size_of::<OpenHow>(),
                        )
                    };
                    if raw < 0 {
                        return Err(invalid());
                    }
                    let mut leaf =
                        unsafe { File::from_raw_fd(i32::try_from(raw).map_err(|_| invalid())?) };
                    require_filesystem(&leaf, PROC_MAGIC, scan)?;
                    let before = actual_object(&leaf, scan)?;
                    if before.kind != libc::S_IFREG
                        || before.dev != object.dev
                        || mount_id(&leaf, scan)? != row.mount
                    {
                        return Err(invalid());
                    }
                    pay(scan, 38, 1)?;
                    let mut raw = Vec::with_capacity(38);
                    loop {
                        let mut bytes = [0_u8; 38];
                        let want = 38 - raw.len();
                        pay(scan, want, 1)?;
                        match leaf.read(&mut bytes[..want]) {
                            Ok(0) => break,
                            Ok(n) => {
                                raw.extend_from_slice(&bytes[..n]);
                                if raw.len() > 37 {
                                    return Err(invalid());
                                }
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                            _ => return Err(invalid()),
                        }
                    }
                    if raw.len() > 37
                        || actual_object(&leaf, scan)? != before
                        || mount_id(&leaf, scan)? != row.mount
                    {
                        return Err(invalid());
                    }
                    drop(leaf);
                    self.revalidate(proc, scan)?;
                    raw
                }
            };
            self.revalidate(proc, scan)?;
            let value = std::str::from_utf8(&raw)
                .map_err(|_| invalid())?
                .strip_suffix('\n')
                .ok_or(invalid())?;
            if !job::uuid(value, false) {
                return Err(invalid());
            }
            Ok(value.to_owned())
        }
    }
    struct TaskNamespace {
        object: NamespaceObject,
        pid: u32,
        start: u64,
        uid_map: Vec<u8>,
        gid_map: Vec<u8>,
    }
    impl TaskNamespace {
        fn observe(proc: &ProcView, pid: u32, scan: &SharedJournalBudget) -> Result<Self> {
            let start = parse_start(
                &proc.read_relative(&format!("{pid}/stat"), 4096, scan)?,
                pid,
            )?;
            let object = proc.user_namespace(pid, scan)?;
            let uid_map = proc.read_relative(&format!("{pid}/uid_map"), 256, scan)?;
            let gid_map = proc.read_relative(&format!("{pid}/gid_map"), 256, scan)?;
            supported_identity_mapping(&uid_map)?;
            supported_identity_mapping(&gid_map)?;
            let value = Self {
                object,
                pid,
                start,
                uid_map,
                gid_map,
            };
            value.revalidate(proc, scan)?;
            Ok(value)
        }
        fn revalidate(&self, proc: &ProcView, scan: &SharedJournalBudget) -> Result<()> {
            self.object.revalidate(scan)?;
            if parse_start(
                &proc.read_relative(&format!("{}/stat", self.pid), 4096, scan)?,
                self.pid,
            )? != self.start
            {
                return Err(invalid());
            }
            let before = proc.user_namespace(self.pid, scan)?;
            if !before.same_object(&self.object) {
                return Err(invalid());
            }
            for (leaf, baseline) in [("uid_map", &self.uid_map), ("gid_map", &self.gid_map)] {
                let raw = proc.read_relative(&format!("{}/{leaf}", self.pid), 256, scan)?;
                supported_identity_mapping(&raw)?;
                if raw != *baseline {
                    return Err(invalid());
                }
            }
            let after = proc.user_namespace(self.pid, scan)?;
            if !after.same_object(&self.object) || !before.same_object(&after) {
                return Err(invalid());
            }
            Ok(())
        }
    }
    struct OwnNativeView {
        proc: ProcView,
        sys: ProcSysView,
        task: TaskNamespace,
    }
    impl OwnNativeView {
        fn observe(scan: &SharedJournalBudget) -> Result<Self> {
            let proc = ProcView::open(scan)?;
            let sys = ProcSysView::observe(&proc, scan)?;
            let task = TaskNamespace::observe(&proc, proc.own_pid, scan)?;
            let view = Self { proc, sys, task };
            view.revalidate(scan)?;
            Ok(view)
        }
        fn revalidate(&self, scan: &SharedJournalBudget) -> Result<()> {
            require_same_current_pid(self.task.pid, scan)?;
            self.proc.revalidate_root(scan)?;
            self.task.revalidate(&self.proc, scan)?;
            self.sys.revalidate(&self.proc, scan)
        }
    }

    // All fixed host ancestors are shared by this burned process, rather than
    // duplicated per library. The cache never replaces an admitted inode.
    struct HeldNamed {
        file: File,
        root: Arc<File>,
        parent: Arc<File>,
        parents: Vec<(String, Stamp)>,
        leaf: CString,
        stamp: Stamp,
        path: String,
    }
    fn share_native(file: File, scan: &SharedJournalBudget) -> Result<Arc<File>> {
        pay(scan, 256, 2)?;
        let m = file.metadata()?;
        let key = (m.dev(), m.ino(), m.mode() & libc::S_IFMT);
        let mut book = NATIVE_FILES
            .get_or_init(|| Mutex::new(BTreeMap::new()))
            .lock()
            .map_err(|_| invalid())?;
        if let Some(original) = book.get(&key).and_then(std::sync::Weak::upgrade) {
            let current = original.metadata()?;
            if current.dev() != m.dev()
                || current.ino() != m.ino()
                || current.mode() != m.mode()
                || current.uid() != m.uid()
                || current.gid() != m.gid()
            {
                return Err(invalid());
            }
            return Ok(original);
        }
        book.retain(|_, v| v.strong_count() > 0);
        if book.len() >= 64 {
            return Err(invalid());
        }
        let original = Arc::new(file);
        book.insert(key, Arc::downgrade(&original));
        Ok(original)
    }
    fn intern_parent(
        path: &str,
        file: File,
        scan: &SharedJournalBudget,
    ) -> Result<(Arc<File>, Stamp)> {
        pay(scan, path.len() * 2 + 256, 2)?;
        let stamp = Stamp::observe(&file, scan)?;
        if stamp.uid != 0 || stamp.mode & libc::S_IFMT != libc::S_IFDIR || stamp.mode & 0o022 != 0 {
            return Err(invalid());
        }
        let mut book = PARENTS
            .get_or_init(|| Mutex::new(BTreeMap::new()))
            .lock()
            .map_err(|_| invalid())?;
        if let Some((original, baseline)) = book
            .get(path)
            .and_then(|(file, stamp)| file.upgrade().map(|file| (file, *stamp)))
        {
            if !stamp.same_directory_identity(&baseline)
                || !Stamp::observe(&original, scan)?.same_directory_identity(&baseline)
            {
                return Err(invalid());
            }
            return Ok((original, baseline));
        }
        // This is a smaller custody bound, never a claim of a measured global
        // descriptor peak. The live descriptor gate below also covers other FDs.
        book.retain(|_, (file, _)| file.strong_count() > 0);
        if book.len() >= 24 {
            return Err(invalid());
        }
        let file = share_native(file, scan)?;
        book.insert(path.to_owned(), (Arc::downgrade(&file), stamp));
        Ok((file, stamp))
    }
    impl HeldNamed {
        fn open(path: &str, scan: &SharedJournalBudget) -> Result<Self> {
            if !job::path(path) {
                return Err(invalid());
            }
            pay(scan, path.len() * 8 + 1024, 0)?;
            pay(scan, 0, path.split('/').count() * 3 + 4)?;
            fd_gate(3, scan)?;
            let raw = unsafe {
                libc::open(
                    c"/".as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if raw < 0 {
                return Err(invalid());
            }
            let root = unsafe { File::from_raw_fd(raw) };
            let mut parent = intern_parent("/", root, scan)?;
            let root = Arc::clone(&parent.0);
            let pieces: Vec<_> = path[1..].split('/').collect();
            let mut parents = Vec::with_capacity(pieces.len());
            let mut prefix = String::new();
            for part in &pieces[..pieces.len() - 1] {
                let name = CString::new(*part).map_err(|_| invalid())?;
                pay(scan, name.as_bytes().len(), 1)?;
                fd_gate(2, scan)?;
                let raw = unsafe {
                    libc::openat(
                        parent.0.as_raw_fd(),
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                };
                if raw < 0 {
                    return Err(invalid());
                }
                parents.push((
                    if prefix.is_empty() {
                        "/".into()
                    } else {
                        prefix.clone()
                    },
                    parent.1,
                ));
                prefix.push('/');
                prefix.push_str(part);
                parent = intern_parent(&prefix, unsafe { File::from_raw_fd(raw) }, scan)?;
            }
            let leaf = CString::new(*pieces.last().ok_or_else(invalid)?).map_err(|_| invalid())?;
            pay(scan, leaf.as_bytes().len(), 1)?;
            let raw = unsafe {
                libc::openat(
                    parent.0.as_raw_fd(),
                    leaf.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                )
            };
            if raw < 0 {
                return Err(invalid());
            }
            let file = unsafe { File::from_raw_fd(raw) };
            let stamp = Stamp::observe(&file, scan)?;
            parents.push((
                if prefix.is_empty() {
                    "/".into()
                } else {
                    prefix
                },
                parent.1,
            ));
            Ok(Self {
                file,
                root,
                parent: parent.0,
                parents,
                leaf,
                stamp,
                path: path.to_owned(),
            })
        }
        fn revalidate(&self, scan: &SharedJournalBudget) -> Result<()> {
            pay(scan, self.path.len() * 8 + 1024, self.parents.len() * 5 + 4)?;
            let mut parent = Arc::clone(&self.root);
            for (index, (path, baseline)) in self.parents.iter().enumerate() {
                if index > 0 {
                    let leaf = path.rsplit('/').next().ok_or_else(invalid)?;
                    let name = CString::new(leaf).map_err(|_| invalid())?;
                    fd_gate(2, scan)?;
                    let raw = unsafe {
                        libc::openat(
                            parent.as_raw_fd(),
                            name.as_ptr(),
                            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                        )
                    };
                    if raw < 0 {
                        return Err(invalid());
                    }
                    parent = intern_parent(path, unsafe { File::from_raw_fd(raw) }, scan)?.0;
                }
                if !Stamp::observe(&parent, scan)?.same_directory_identity(baseline) {
                    return Err(invalid());
                }
            }
            if !Stamp::observe(&self.parent, scan)?
                .same_directory_identity(&self.parents.last().ok_or_else(invalid)?.1)
                || actual_object(&parent, scan)? != actual_object(&self.parent, scan)?
            {
                return Err(invalid());
            }
            fd_gate(2, scan)?;
            pay(scan, self.leaf.as_bytes().len() + 1, 2)?;
            let raw = unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    self.leaf.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                )
            };
            if raw < 0 {
                return Err(invalid());
            }
            let named = unsafe { File::from_raw_fd(raw) };
            if Stamp::observe(&named, scan)? != self.stamp {
                return Err(invalid());
            }
            if Stamp::observe(&self.file, scan)? != self.stamp {
                return Err(invalid());
            }
            pay(scan, 0, 1)?;
            require_cloexec(self.file.as_raw_fd())
        }
        fn full_read_and_hash(
            &mut self,
            maximum: u64,
            scan: &SharedJournalBudget,
        ) -> Result<(Vec<u8>, [u8; 32])> {
            if !self.stamp.root_file(maximum) {
                return Err(invalid());
            }
            self.revalidate(scan)?;
            let bytes = read_exact_file(&mut self.file, self.stamp.bytes, scan)?;
            let digest = digest_bytes(&bytes, scan)?;
            self.revalidate(scan)?;
            Ok((bytes, digest))
        }
    }
    fn digest_bytes(raw: &[u8], scan: &SharedJournalBudget) -> Result<[u8; 32]> {
        pay(scan, raw.len() + 32, 0)?;
        Ok(Sha256::digest(raw).into())
    }
    fn read_exact_file(
        file: &mut File,
        length: u64,
        scan: &SharedJournalBudget,
    ) -> Result<Vec<u8>> {
        let length = usize::try_from(length).map_err(|_| invalid())?;
        pay(scan, length, 1)?;
        let mut raw = vec![0; length];
        pay(scan, 0, 1)?;
        file.seek(SeekFrom::Start(0))?;
        let mut used = 0;
        while used < raw.len() {
            pay(scan, raw.len() - used, 1)?;
            match file.read(&mut raw[used..]) {
                Ok(0) => return Err(invalid()),
                Ok(n) => used += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(invalid()),
            }
        }
        let mut tail = [0];
        loop {
            pay(scan, 1, 1)?;
            match file.read(&mut tail) {
                Ok(0) => break,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                _ => return Err(invalid()),
            }
        }
        Ok(raw)
    }
    fn fd_gate(additional: usize, scan: &SharedJournalBudget) -> Result<()> {
        // The fixed getdents bound includes its iterator FD and cap+1. This is
        // an actual local observation, not a structural global-FD proof.
        pay(scan, 8192, 70)?;
        let raw = unsafe {
            libc::open(
                c"/proc/self/fd".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if raw < 0 {
            return Err(invalid());
        }
        let directory = unsafe { File::from_raw_fd(raw) };
        let mut buffer = [0_u8; 4096];
        let mut count = 0_usize;
        loop {
            pay(scan, buffer.len(), 1)?;
            let n = unsafe {
                libc::syscall(
                    libc::SYS_getdents64,
                    directory.as_raw_fd(),
                    buffer.as_mut_ptr(),
                    buffer.len(),
                )
            };
            if n < 0 {
                return Err(invalid());
            }
            if n == 0 {
                break;
            }
            let mut offset = 0;
            while offset < n as usize {
                if offset + 19 > n as usize {
                    return Err(invalid());
                }
                let size = u16::from_ne_bytes([buffer[offset + 16], buffer[offset + 17]]) as usize;
                if size < 20 || offset + size > n as usize {
                    return Err(invalid());
                }
                let bytes = &buffer[offset + 19..offset + size];
                let end = bytes.iter().position(|b| *b == 0).ok_or_else(invalid)?;
                if &bytes[..end] != b"." && &bytes[..end] != b".." {
                    count += 1;
                }
                if count > 64 {
                    return Err(invalid());
                }
                offset += size;
            }
        }
        // The iterator is already included in the observed peak, but closes
        // on return before the requested owner/probe opens are attempted.
        if count
            .checked_sub(1)
            .and_then(|n| n.checked_add(additional))
            .is_none_or(|n| n > 64)
        {
            return Err(invalid());
        }
        Ok(())
    }
    fn encode<T: Serialize>(value: &T, cap: usize, scan: &SharedJournalBudget) -> Result<Vec<u8>> {
        let mut count = ObservationCounter { bytes: 0, scan };
        serde_json::to_writer(&mut count, value)?;
        if count.bytes == 0 || count.bytes > cap {
            return Err(invalid());
        }
        pay(scan, count.bytes.checked_mul(64).ok_or_else(invalid)?, 1)?;
        let bytes = registry::canonical(value)?;
        if bytes.len() != count.bytes || bytes.len() > cap {
            return Err(invalid());
        }
        Ok(bytes)
    }
    fn now_ms(scan: &SharedJournalBudget) -> Result<u64> {
        pay(scan, 0, 1)?;
        u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| invalid())?
                .as_millis(),
        )
        .map_err(|_| invalid())
    }
    fn monotonic_ns(scan: &SharedJournalBudget) -> Result<u64> {
        pay(scan, 0, 1)?;
        let mut time = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } != 0 {
            return Err(invalid());
        }
        u64::try_from(time.tv_sec)
            .ok()
            .and_then(|v| v.checked_mul(1_000_000_000))
            .and_then(|v| v.checked_add(u64::try_from(time.tv_nsec).ok()?))
            .filter(|v| *v > 0)
            .ok_or_else(invalid)
    }
    struct FixedRecord<T> {
        named: HeldNamed,
        raw: Vec<u8>,
        digest: [u8; 32],
        data: T,
    }
    impl<T: Record + DeserializeOwned> FixedRecord<T> {
        fn open(path: &str, cap: usize, scan: &SharedJournalBudget) -> Result<Self> {
            let mut named = HeldNamed::open(path, scan)?;
            if named.stamp.mode & 0o7777 != 0o600
                || named.stamp.gid != 0
                || named
                    .parents
                    .last()
                    .is_none_or(|p| p.1.mode & 0o7777 != 0o700)
            {
                return Err(invalid());
            }
            let (raw, digest) = named.full_read_and_hash(cap as u64, scan)?;
            let data: T = parse(&raw, cap, scan)?;
            data.validate()?;
            Ok(Self {
                named,
                raw,
                digest,
                data,
            })
        }
        fn recheck(&mut self, scan: &SharedJournalBudget) -> Result<()> {
            let (raw, digest) = self.named.full_read_and_hash(self.raw.len() as u64, scan)?;
            if raw != self.raw || digest != self.digest {
                return Err(invalid());
            }
            Ok(())
        }
    }
    fn open_beneath(
        root: &File,
        path: &str,
        flags: i32,
        scan: &SharedJournalBudget,
    ) -> Result<File> {
        if !job::path(path) {
            return Err(invalid());
        }
        fd_gate(2, scan)?;
        pay(scan, path.len() * 2 + 128, 2)?;
        let relative = CString::new(&path[1..]).map_err(|_| invalid())?;
        let how = OpenHow {
            flags: (flags | libc::O_CLOEXEC | libc::O_NOFOLLOW) as u64,
            mode: 0,
            resolve: RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS,
        };
        let raw = unsafe {
            libc::syscall(
                libc::SYS_openat2,
                root.as_raw_fd(),
                relative.as_ptr(),
                &how,
                std::mem::size_of::<OpenHow>(),
            )
        };
        if raw < 0 {
            return Err(invalid());
        }
        Ok(unsafe { File::from_raw_fd(i32::try_from(raw).map_err(|_| invalid())?) })
    }
    struct CodeBinding {
        root: Arc<File>,
        root_stamp: Stamp,
        ancestors: Vec<(String, Stamp)>,
        leaf: CString,
        path: String,
    }
    fn code_directory(stamp: Stamp) -> Result<()> {
        if stamp.uid != 0 || stamp.mode & libc::S_IFMT != libc::S_IFDIR || stamp.mode & 0o022 != 0 {
            return Err(invalid());
        }
        Ok(())
    }
    impl CodeBinding {
        fn open(root: Arc<File>, path: &str, scan: &SharedJournalBudget) -> Result<Self> {
            pay(scan, path.len() * 32 + 8192, 64)?;
            if !job::path(path) || path.split('/').count() > 16 {
                return Err(invalid());
            }
            let root_stamp = Stamp::observe(&root, scan)?;
            code_directory(root_stamp)?;
            let pieces: Vec<_> = path[1..].split('/').collect();
            let mut parent = Arc::clone(&root);
            let mut ancestors = Vec::with_capacity(pieces.len());
            for part in &pieces[..pieces.len() - 1] {
                fd_gate(2, scan)?;
                pay(scan, part.len() + 128, 2)?;
                let name = CString::new(*part).map_err(|_| invalid())?;
                let raw = unsafe {
                    libc::openat(
                        parent.as_raw_fd(),
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                    )
                };
                if raw < 0 {
                    return Err(invalid());
                }
                let next = share_native(unsafe { File::from_raw_fd(raw) }, scan)?;
                let stamp = Stamp::observe(&next, scan)?;
                code_directory(stamp)?;
                ancestors.push(((*part).to_owned(), stamp));
                parent = next;
            }
            let leaf = CString::new(*pieces.last().ok_or_else(invalid)?).map_err(|_| invalid())?;
            drop(parent);
            Ok(Self {
                root,
                root_stamp,
                ancestors,
                leaf,
                path: path.to_owned(),
            })
        }
        fn open_leaf(&self, scan: &SharedJournalBudget) -> Result<File> {
            let parent = self.parent_checked(scan)?;
            fd_gate(2, scan)?;
            pay(scan, self.leaf.as_bytes().len(), 1)?;
            let raw = unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    self.leaf.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                )
            };
            if raw < 0 {
                return Err(invalid());
            }
            Ok(unsafe { File::from_raw_fd(raw) })
        }
        fn parent_checked(&self, scan: &SharedJournalBudget) -> Result<Arc<File>> {
            pay(
                scan,
                self.path.len() * 16 + 4096,
                self.ancestors.len() * 4 + 2,
            )?;
            let stamp = Stamp::observe(&self.root, scan)?;
            code_directory(stamp)?;
            if stamp != self.root_stamp {
                return Err(invalid());
            }
            let mut current = Arc::clone(&self.root);
            for (part, baseline) in &self.ancestors {
                fd_gate(2, scan)?;
                pay(scan, part.len() + 1, 1)?;
                let name = CString::new(part.as_str()).map_err(|_| invalid())?;
                let raw = unsafe {
                    libc::openat(
                        current.as_raw_fd(),
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                    )
                };
                if raw < 0 {
                    return Err(invalid());
                }
                let next = share_native(unsafe { File::from_raw_fd(raw) }, scan)?;
                let now = Stamp::observe(&next, scan)?;
                code_directory(now)?;
                if now != *baseline {
                    return Err(invalid());
                }
                current = next;
            }
            Ok(current)
        }
    }
    struct MeasuredCode {
        file: File,
        binding: CodeBinding,
        path: String,
        stamp: Stamp,
        digest: [u8; 32],
    }
    impl MeasuredCode {
        fn open(
            root: Arc<File>,
            declared: &job::CodeFile,
            manager: bool,
            scan: &SharedJournalBudget,
        ) -> Result<Self> {
            declared.validate()?;
            let binding = CodeBinding::open(root, &declared.path, scan)?;
            let mut file = binding.open_leaf(scan)?;
            let stamp = Stamp::observe(&file, scan)?;
            if !stamp.same_declared(&declared.stamp)
                || !stamp.root_file(128 * 1024 * 1024)
                || (manager && stamp.mode & 0o7777 != 0o500)
            {
                return Err(invalid());
            }
            let raw = read_exact_file(&mut file, stamp.bytes, scan)?;
            let digest = digest_bytes(&raw, scan)?;
            if !raw.starts_with(b"\x7fELF") || hex::encode(digest) != declared.sha256 {
                return Err(invalid());
            }
            pay(scan, declared.path.len(), 1)?;
            let value = Self {
                file,
                binding,
                path: declared.path.clone(),
                stamp,
                digest,
            };
            value.recheck(scan)?;
            Ok(value)
        }
        fn recheck(&self, scan: &SharedJournalBudget) -> Result<()> {
            let named = self.binding.open_leaf(scan)?;
            if Stamp::observe(&self.file, scan)? != self.stamp
                || Stamp::observe(&named, scan)? != self.stamp
            {
                return Err(invalid());
            }
            Ok(())
        }
    }
    struct MeasuredProcess {
        pid: u32,
        start: u64,
        namespace_pid: u32,
        root: Arc<File>,
        root_object: NativeObject,
        user: TaskNamespace,
        pidns: Arc<File>,
        mntns: Arc<File>,
        pid_name: String,
        mnt_name: String,
        codes: Vec<Arc<MeasuredCode>>,
        code_views: Vec<CodeBinding>,
        mappings: Vec<String>,
        pidfd: Option<Arc<ParentPid>>,
    }
    fn process_status(
        proc: &ProcView,
        pid: u32,
        uid: u32,
        scan: &SharedJournalBudget,
    ) -> Result<u32> {
        let (actual, namespace_pid) = process_identity_status(proc, pid, scan)?;
        if actual != uid {
            return Err(invalid());
        }
        Ok(namespace_pid)
    }
    fn process_identity_status(
        proc: &ProcView,
        pid: u32,
        scan: &SharedJournalBudget,
    ) -> Result<(u32, u32)> {
        let raw = proc.read_relative(&format!("{pid}/status"), 65536, scan)?;
        pay(scan, raw.len() * 8 + 1024, 64)?;
        let value = std::str::from_utf8(&raw).map_err(|_| invalid())?;
        let mut user = None;
        let mut group = None;
        let mut nspid = None;
        for line in value.lines() {
            pay(scan, 0, 1)?;
            if line.starts_with("Uid:") || line.starts_with("Gid:") {
                let mut fields = line.split_ascii_whitespace().skip(1);
                let mut values = [0_u32; 4];
                for slot in &mut values {
                    *slot = fields
                        .next()
                        .ok_or_else(invalid)?
                        .parse()
                        .map_err(|_| invalid())?;
                }
                if fields.next().is_some() || values.iter().any(|v| *v != values[0]) {
                    return Err(invalid());
                }
                let slot = if line.starts_with("Uid:") {
                    &mut user
                } else {
                    &mut group
                };
                if slot.replace(values[0]).is_some() {
                    return Err(invalid());
                }
            }
            if let Some(tail) = line.strip_prefix("NSpid:") {
                if nspid.is_some() {
                    return Err(invalid());
                }
                let mut values = [0_u32; 32];
                let mut count = 0_usize;
                for item in tail.split_ascii_whitespace() {
                    if count == values.len() {
                        return Err(invalid());
                    }
                    values[count] = item.parse().map_err(|_| invalid())?;
                    count += 1;
                }
                if count == 0 || values[0] != pid || values[..count].contains(&0) {
                    return Err(invalid());
                }
                nspid = Some(values[count - 1]);
            }
        }
        let user = user.ok_or_else(invalid)?;
        if group != Some(user) {
            return Err(invalid());
        }
        Ok((user, nspid.ok_or_else(invalid)?))
    }
    fn executable_mappings(
        raw: &[u8],
        codes: &[Arc<MeasuredCode>],
        main: &str,
        scan: &SharedJournalBudget,
    ) -> Result<Vec<String>> {
        pay(scan, raw.len() * 8 + 1024, 1)?;
        let text = std::str::from_utf8(raw).map_err(|_| invalid())?;
        let mut result = Vec::new();
        let (mut vdso, mut vsys) = (false, false);
        let mut seen = BTreeSet::new();
        for line in text.lines() {
            pay(scan, 0, 8)?;
            let mut fields = line.split_ascii_whitespace();
            let mut f = [""; 6];
            for slot in &mut f[..5] {
                *slot = fields.next().ok_or_else(invalid)?;
            }
            if f[1].len() != 4 {
                return Err(invalid());
            }
            if f[1].as_bytes()[2] != b'x' {
                continue;
            }
            f[5] = fields.next().ok_or_else(invalid)?;
            if fields.next().is_some() || f[1].as_bytes()[1] == b'w' || f[1].as_bytes()[3] != b'p' {
                return Err(invalid());
            }
            let (a, b) = f[0].split_once('-').ok_or_else(invalid)?;
            let a = u64::from_str_radix(a, 16).map_err(|_| invalid())?;
            let b = u64::from_str_radix(b, 16).map_err(|_| invalid())?;
            if a == 0 || a >= b {
                return Err(invalid());
            }
            match f[5] {
                "[vdso]" => {
                    if vdso {
                        return Err(invalid());
                    }
                    vdso = true;
                }
                "[vsyscall]" => {
                    if vsys {
                        return Err(invalid());
                    }
                    vsys = true;
                }
                path => {
                    let code = codes.iter().find(|c| c.path == path).ok_or_else(invalid)?;
                    let (major, minor) = f[3].split_once(':').ok_or_else(invalid)?;
                    if u32::from_str_radix(major, 16).map_err(|_| invalid())?
                        != libc::major(code.stamp.dev)
                        || u32::from_str_radix(minor, 16).map_err(|_| invalid())?
                            != libc::minor(code.stamp.dev)
                        || f[4].parse::<u64>().map_err(|_| invalid())? != code.stamp.ino
                        || u64::from_str_radix(f[2], 16).map_err(|_| invalid())? >= code.stamp.bytes
                    {
                        return Err(invalid());
                    }
                    seen.insert(path.to_owned());
                }
            }
            result.push(line.to_owned());
        }
        if result.is_empty() || !seen.contains(main) {
            return Err(invalid());
        }
        Ok(result)
    }
    impl MeasuredProcess {
        fn open(
            proc: &ProcView,
            pid: u32,
            row: &job::RoleImage,
            cache: &mut Vec<Arc<MeasuredCode>>,
            scan: &SharedJournalBudget,
        ) -> Result<Self> {
            Self::open_with_original_pidfd(proc, pid, row, cache, scan, None)
        }
        fn open_with_original_pidfd(
            proc: &ProcView,
            pid: u32,
            row: &job::RoleImage,
            cache: &mut Vec<Arc<MeasuredCode>>,
            scan: &SharedJournalBudget,
            original: Option<&Arc<ParentPid>>,
        ) -> Result<Self> {
            if row.libraries.len() > 32 {
                return Err(invalid());
            }
            pay(scan, 16384, 32)?;
            let start = parse_start(
                &proc.read_relative(&format!("{pid}/stat"), 4096, scan)?,
                pid,
            )?;
            let namespace_pid = process_status(proc, pid, 0, scan)?;
            let root = share_native(proc.open_checked_magic(&format!("{pid}/root"), scan)?, scan)?;
            let root_object = actual_object(&root, scan)?;
            if root_object.kind != libc::S_IFDIR {
                return Err(invalid());
            }
            let user = TaskNamespace::observe(proc, pid, scan)?;
            let pid_name = proc.read_link_relative(&format!("{pid}/ns/pid"), scan)?;
            let mnt_name = proc.read_link_relative(&format!("{pid}/ns/mnt"), scan)?;
            let pidns = share_native(
                proc.open_checked_magic(&format!("{pid}/ns/pid"), scan)?,
                scan,
            )?;
            let mntns = share_native(
                proc.open_checked_magic(&format!("{pid}/ns/mnt"), scan)?,
                scan,
            )?;
            for (file, kind) in [(&pidns, libc::CLONE_NEWPID), (&mntns, libc::CLONE_NEWNS)] {
                require_filesystem(file, NSFS_MAGIC, scan)?;
                pay(scan, 0, 1)?;
                if unsafe { libc::ioctl(file.as_raw_fd(), NS_GET_NSTYPE) } != kind {
                    return Err(invalid());
                }
            }
            let mut codes = Vec::new();
            let mut code_views = Vec::new();
            for (index, declared) in std::iter::once(&row.executable)
                .chain(row.libraries.iter())
                .enumerate()
            {
                let view = CodeBinding::open(Arc::clone(&root), &declared.path, scan)?;
                let current = view.open_leaf(scan)?;
                if !Stamp::observe(&current, scan)?.same_declared(&declared.stamp) {
                    return Err(invalid());
                }
                drop(current);
                code_views.push(view);
                if let Some(code) = cache.iter().find(|c| {
                    c.stamp.same_declared(&declared.stamp)
                        && hex::encode(c.digest) == declared.sha256
                }) {
                    codes.push(Arc::clone(code));
                    continue;
                }
                if cache.len() >= 64 {
                    return Err(invalid());
                }
                let code = Arc::new(MeasuredCode::open(
                    Arc::clone(&root),
                    declared,
                    index == 0,
                    scan,
                )?);
                cache.push(Arc::clone(&code));
                codes.push(code);
            }
            let actual = proc.open_checked_magic(&format!("{pid}/exe"), scan)?;
            if Stamp::observe(&actual, scan)? != codes[0].stamp {
                return Err(invalid());
            }
            let mappings = executable_mappings(
                &proc.read_relative(&format!("{pid}/maps"), 1024 * 1024, scan)?,
                &codes,
                &row.executable.path,
                scan,
            )?;
            let pidfd = if pid == proc.own_pid {
                if original.is_some() {
                    return Err(invalid());
                }
                None
            } else if let Some(original) = original {
                original.revalidate(scan)?;
                if original.pid != pid || original.start != start {
                    return Err(invalid());
                }
                Some(Arc::clone(original))
            } else {
                Some(Arc::new(ParentPid::open(pid, start, scan)?))
            };
            let value = Self {
                pid,
                start,
                namespace_pid,
                root,
                root_object,
                user,
                pidns,
                mntns,
                pid_name,
                mnt_name,
                codes,
                code_views,
                mappings,
                pidfd,
            };
            value.recheck(proc, scan)?;
            Ok(value)
        }
        fn recheck(&self, proc: &ProcView, scan: &SharedJournalBudget) -> Result<()> {
            if let Some(pidfd) = &self.pidfd {
                pidfd.revalidate(scan)?;
            } else {
                require_same_current_pid(self.pid, scan)?;
            }
            self.user.revalidate(proc, scan)?;
            if process_status(proc, self.pid, 0, scan)? != self.namespace_pid
                || parse_start(
                    &proc.read_relative(&format!("{}/stat", self.pid), 4096, scan)?,
                    self.pid,
                )? != self.start
            {
                return Err(invalid());
            }
            let root = proc.open_checked_magic(&format!("{}/root", self.pid), scan)?;
            if actual_object(&self.root, scan)? != self.root_object
                || actual_object(&root, scan)? != self.root_object
            {
                return Err(invalid());
            }
            drop(root);
            for (leaf, file, name) in [
                ("pid", &self.pidns, &self.pid_name),
                ("mnt", &self.mntns, &self.mnt_name),
            ] {
                let named = proc.open_checked_magic(&format!("{}/ns/{leaf}", self.pid), scan)?;
                if actual_object(file, scan)? != actual_object(&named, scan)?
                    || proc.read_link_relative(&format!("{}/ns/{leaf}", self.pid), scan)? != *name
                {
                    return Err(invalid());
                }
            }
            for (code, view) in self.codes.iter().zip(&self.code_views) {
                code.recheck(scan)?;
                let named = view.open_leaf(scan)?;
                if Stamp::observe(&named, scan)? != code.stamp {
                    return Err(invalid());
                }
            }
            if Stamp::observe(
                &proc.open_checked_magic(&format!("{}/exe", self.pid), scan)?,
                scan,
            )? != self.codes[0].stamp
                || executable_mappings(
                    &proc.read_relative(&format!("{}/maps", self.pid), 1024 * 1024, scan)?,
                    &self.codes,
                    &self.codes[0].path,
                    scan,
                )? != self.mappings
            {
                return Err(invalid());
            }
            Ok(())
        }
        fn record(&self, scan: &SharedJournalBudget) -> Result<job::ProcessRef> {
            pay(scan, 1024, 1)?;
            Ok(job::ProcessRef {
                host_pid: u64::from(self.pid),
                namespace_pid: u64::from(self.namespace_pid),
                start_ticks: self.start,
                pid_namespace: self.pid_name.clone(),
                mount_namespace: self.mnt_name.clone(),
                exe_sha256: hex::encode(self.codes[0].digest),
                exe_dev: self.codes[0].stamp.dev,
                exe_ino: self.codes[0].stamp.ino,
                real_uid: 0,
                effective_uid: 0,
                saved_uid: 0,
                real_gid: 0,
                effective_gid: 0,
                saved_gid: 0,
            })
        }
    }
    // Fixed Engine API 1.45. The 101 + multiplexed-stream negotiation and
    // eight-byte stdout/stderr framing follow Moby v26.1.5 exec.go/hijack.go/
    // stdcopy.go. No HTTP/URL/command/exec-id API is exposed outside this job.
    struct StrictJson(serde_json::Value);
    impl<'de> Deserialize<'de> for StrictJson {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
            struct V;
            impl<'de> serde::de::Visitor<'de> for V {
                type Value = StrictJson;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str("bounded JSON without duplicate fields")
                }
                fn visit_bool<E: serde::de::Error>(
                    self,
                    v: bool,
                ) -> std::result::Result<Self::Value, E> {
                    Ok(StrictJson(v.into()))
                }
                fn visit_i64<E: serde::de::Error>(
                    self,
                    v: i64,
                ) -> std::result::Result<Self::Value, E> {
                    Ok(StrictJson(v.into()))
                }
                fn visit_u64<E: serde::de::Error>(
                    self,
                    v: u64,
                ) -> std::result::Result<Self::Value, E> {
                    Ok(StrictJson(v.into()))
                }
                fn visit_f64<E: serde::de::Error>(
                    self,
                    _: f64,
                ) -> std::result::Result<Self::Value, E> {
                    Err(E::custom("integer JSON only"))
                }
                fn visit_str<E: serde::de::Error>(
                    self,
                    v: &str,
                ) -> std::result::Result<Self::Value, E> {
                    Ok(StrictJson(v.into()))
                }
                fn visit_string<E: serde::de::Error>(
                    self,
                    v: String,
                ) -> std::result::Result<Self::Value, E> {
                    Ok(StrictJson(v.into()))
                }
                fn visit_none<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
                    Ok(StrictJson(serde_json::Value::Null))
                }
                fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
                    Ok(StrictJson(serde_json::Value::Null))
                }
                fn visit_seq<A: serde::de::SeqAccess<'de>>(
                    self,
                    mut a: A,
                ) -> std::result::Result<Self::Value, A::Error> {
                    let mut v = Vec::new();
                    while let Some(x) = a.next_element::<StrictJson>()? {
                        if v.len() >= ENTRY_CAP {
                            return Err(serde::de::Error::custom("JSON entries"));
                        }
                        v.push(x.0);
                    }
                    Ok(StrictJson(v.into()))
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(
                    self,
                    mut a: A,
                ) -> std::result::Result<Self::Value, A::Error> {
                    let mut v = serde_json::Map::new();
                    while let Some(k) = a.next_key::<String>()? {
                        if v.len() >= ENTRY_CAP || v.contains_key(&k) {
                            return Err(serde::de::Error::custom("JSON duplicate or entries"));
                        }
                        v.insert(k, a.next_value::<StrictJson>()?.0);
                    }
                    Ok(StrictJson(v.into()))
                }
            }
            d.deserialize_any(V)
        }
    }
    fn api_json(raw: &[u8], cap: usize, scan: &SharedJournalBudget) -> Result<serde_json::Value> {
        if raw.is_empty() || raw.len() > cap {
            return Err(invalid());
        }
        pay(scan, raw.len() * 64 + 1024, 0)?;
        let nodes = raw
            .iter()
            .filter(|b| matches!(**b, b'{' | b'[' | b',' | b':'))
            .count()
            + 1;
        pay(scan, 0, nodes)?;
        Ok(serde_json::from_slice::<StrictJson>(raw)?.0)
    }
    // Engine replies have many fields outside the approved observation. Typed
    // projection skips them during decoding: Config.Env/Command and unrelated
    // labels are never materialized, hashed, logged or published as JSON DATA.
    #[derive(Serialize, Deserialize)]
    struct SafeLabels {
        #[serde(rename = "com.docker.compose.project")]
        project: Option<String>,
        #[serde(rename = "com.docker.compose.service")]
        service: Option<String>,
    }
    #[derive(Serialize, Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct SafeState {
        running: bool,
        pid: u64,
        started_at: String,
    }
    #[derive(Serialize, Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct SafeConfig {
        image: String,
        user: String,
        labels: Option<SafeLabels>,
    }
    #[derive(Serialize, Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct SafeHost {
        privileged: bool,
        publish_all_ports: bool,
        pid_mode: String,
        userns_mode: String,
        network_mode: String,
        port_bindings: serde_json::Value,
    }
    #[derive(Serialize, Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct SafeContainerNet {
        ports: serde_json::Value,
        networks: serde_json::Value,
    }
    #[derive(Serialize, Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct SafeContainer {
        id: String,
        image: String,
        state: SafeState,
        restart_count: u64,
        config: SafeConfig,
        mounts: Vec<ActualAssetsMount>,
        host_config: SafeHost,
        network_settings: SafeContainerNet,
    }
    #[derive(Serialize, Deserialize)]
    struct SafeId {
        #[serde(rename = "Id")]
        id: String,
    }
    #[derive(Serialize, Deserialize)]
    struct SafeInfo {
        #[serde(rename = "ID")]
        id: String,
    }
    #[derive(Serialize, Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct SafeImage {
        id: String,
        repo_digests: Vec<String>,
    }
    #[derive(Serialize, Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct SafeNetwork {
        id: String,
        internal: bool,
        scope: String,
        driver: String,
        labels: Option<SafeLabels>,
        containers: serde_json::Value,
    }
    fn projected_reply(
        path: &str,
        raw: &[u8],
        cap: usize,
        scan: &SharedJournalBudget,
    ) -> Result<serde_json::Value> {
        if raw.is_empty() || raw.len() > cap {
            return Err(invalid());
        }
        pay(scan, raw.len() * 64 + 4096, 0)?;
        let nodes = raw
            .iter()
            .filter(|b| matches!(**b, b'{' | b'[' | b',' | b':'))
            .count()
            + 1;
        pay(scan, 0, nodes)?;
        if path == "/v1.45/info" {
            Ok(serde_json::to_value(serde_json::from_slice::<SafeInfo>(
                raw,
            )?)?)
        } else if path == "/v1.45/containers/json?all=1&size=0" {
            let rows: Vec<SafeId> = serde_json::from_slice(raw)?;
            if rows.len() > CONTAINER_CAP {
                return Err(invalid());
            }
            Ok(serde_json::to_value(rows)?)
        } else if path.starts_with("/v1.45/containers/") && path.ends_with("/json") {
            Ok(serde_json::to_value(serde_json::from_slice::<
                SafeContainer,
            >(raw)?)?)
        } else if path.starts_with("/v1.45/images/") && path.ends_with("/json") {
            Ok(serde_json::to_value(serde_json::from_slice::<SafeImage>(
                raw,
            )?)?)
        } else if path.starts_with("/v1.45/networks/") {
            Ok(serde_json::to_value(
                serde_json::from_slice::<SafeNetwork>(raw)?,
            )?)
        } else {
            api_json(raw, cap, scan)
        }
    }
    fn jstr<'a>(v: &'a serde_json::Value, key: &str, cap: usize) -> Result<&'a str> {
        let s = v.get(key).and_then(|x| x.as_str()).ok_or_else(invalid)?;
        if !text(s, cap) {
            return Err(invalid());
        }
        Ok(s)
    }
    fn ju64(v: &serde_json::Value, key: &str) -> Result<u64> {
        v.get(key).and_then(|x| x.as_u64()).ok_or_else(invalid)
    }
    fn jbool(v: &serde_json::Value, key: &str) -> Result<bool> {
        v.get(key).and_then(|x| x.as_bool()).ok_or_else(invalid)
    }
    fn peer(socket: &UnixStream, scan: &SharedJournalBudget) -> Result<(u32, u64)> {
        pay(scan, 0, 2)?;
        require_cloexec(socket.as_raw_fd())?;
        let mut cred = libc::ucred {
            pid: 0,
            uid: u32::MAX,
            gid: u32::MAX,
        };
        let mut size = std::mem::size_of_val(&cred) as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut cred as *mut libc::ucred).cast(),
                &mut size,
            )
        } != 0
            || size as usize != std::mem::size_of_val(&cred)
            || cred.uid != 0
            || cred.gid != 0
            || cred.pid <= 0
        {
            return Err(invalid());
        }
        let mut cookie = 0_u64;
        let mut size = std::mem::size_of_val(&cookie) as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_COOKIE,
                (&mut cookie as *mut u64).cast(),
                &mut size,
            )
        } != 0
            || size as usize != std::mem::size_of_val(&cookie)
            || cookie == 0
        {
            return Err(invalid());
        }
        Ok((cred.pid as u32, cookie))
    }
    fn connect_daemon(deadline: Instant, scan: &SharedJournalBudget) -> Result<UnixStream> {
        fd_gate(2, scan)?;
        pay(scan, 256, 3)?;
        let raw = unsafe {
            libc::socket(
                libc::AF_UNIX,
                libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                0,
            )
        };
        if raw < 0 {
            return Err(invalid());
        }
        let socket = unsafe { UnixStream::from_raw_fd(raw) };
        let mut address = unsafe { std::mem::zeroed::<libc::sockaddr_un>() };
        address.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (to, from) in address.sun_path.iter_mut().zip(DOCKER_SOCKET.as_bytes()) {
            *to = *from as libc::c_char;
        }
        let result = unsafe {
            libc::connect(
                raw,
                (&address as *const libc::sockaddr_un).cast(),
                std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
            )
        };
        if result != 0 {
            let e = std::io::Error::last_os_error();
            if !matches!(
                e.raw_os_error(),
                Some(libc::EINPROGRESS) | Some(libc::EAGAIN)
            ) {
                return Err(invalid());
            }
            root_socket_poll(&socket, libc::POLLOUT, deadline, &STOP, scan)?;
        }
        let mut error = 0_i32;
        let mut size = std::mem::size_of_val(&error) as libc::socklen_t;
        pay(scan, 0, 1)?;
        if unsafe {
            libc::getsockopt(
                raw,
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                (&mut error as *mut i32).cast(),
                &mut size,
            )
        } != 0
            || error != 0
        {
            return Err(invalid());
        }
        Ok(socket)
    }
    fn socket_read_some(
        socket: &UnixStream,
        raw: &mut [u8],
        deadline: Instant,
        scan: &SharedJournalBudget,
    ) -> Result<usize> {
        loop {
            root_io_boundary(deadline, &STOP)?;
            pay(scan, raw.len(), 1)?;
            let n = unsafe {
                libc::recv(
                    socket.as_raw_fd(),
                    raw.as_mut_ptr().cast(),
                    raw.len(),
                    libc::MSG_DONTWAIT,
                )
            };
            if n >= 0 {
                return Ok(n as usize);
            }
            match std::io::Error::last_os_error().kind() {
                std::io::ErrorKind::Interrupted => continue,
                std::io::ErrorKind::WouldBlock => {
                    root_socket_poll(socket, libc::POLLIN, deadline, &STOP, scan)?
                }
                _ => return Err(invalid()),
            }
        }
    }
    struct HttpHead {
        status: u16,
        length: Option<usize>,
        chunked: bool,
        multiplexed: bool,
        upgrade: bool,
    }
    fn http_head(
        socket: &UnixStream,
        deadline: Instant,
        scan: &SharedJournalBudget,
    ) -> Result<HttpHead> {
        pay(scan, 8192, 1)?;
        let mut raw = Vec::with_capacity(8192);
        while !raw.ends_with(b"\r\n\r\n") {
            if raw.len() >= 8192 {
                return Err(invalid());
            }
            let mut b = [0];
            root_socket_exact(socket, &mut b, deadline, &STOP, scan)?;
            raw.push(b[0]);
        }
        pay(scan, raw.len() * 8, 64)?;
        let text = std::str::from_utf8(&raw).map_err(|_| invalid())?;
        let mut lines = text.split("\r\n");
        let status = lines.next().ok_or_else(invalid)?;
        let mut words = status.splitn(3, ' ');
        let protocol = words.next().ok_or_else(invalid)?;
        let code = words.next().ok_or_else(invalid)?;
        let _reason = words.next().ok_or_else(invalid)?;
        if protocol != "HTTP/1.1" {
            return Err(invalid());
        }
        let status = code.parse::<u16>().map_err(|_| invalid())?;
        let (mut length, mut chunked, mut multiplexed, mut upgrade) = (None, false, false, false);
        let mut seen = BTreeSet::new();
        for line in lines {
            if line.is_empty() {
                continue;
            }
            pay(scan, 0, 2)?;
            let (k, v) = line.split_once(':').ok_or_else(invalid)?;
            let k = k.to_ascii_lowercase();
            if !seen.insert(k.clone()) {
                return Err(invalid());
            }
            let v = v.trim();
            match k.as_str() {
                "content-length" => {
                    length = Some(v.parse::<usize>().map_err(|_| invalid())?);
                }
                "transfer-encoding" => {
                    if v != "chunked" {
                        return Err(invalid());
                    }
                    chunked = true;
                }
                "content-type" => {
                    multiplexed = v == "application/vnd.docker.multiplexed-stream";
                }
                "upgrade" => {
                    upgrade = v.eq_ignore_ascii_case("tcp");
                }
                _ => {}
            }
        }
        if length.is_some() && chunked {
            return Err(invalid());
        }
        Ok(HttpHead {
            status,
            length,
            chunked,
            multiplexed,
            upgrade,
        })
    }
    fn line(
        socket: &UnixStream,
        cap: usize,
        deadline: Instant,
        scan: &SharedJournalBudget,
    ) -> Result<Vec<u8>> {
        pay(scan, cap, 1)?;
        let mut raw = Vec::with_capacity(cap);
        while !raw.ends_with(b"\r\n") {
            if raw.len() >= cap {
                return Err(invalid());
            }
            let mut b = [0];
            root_socket_exact(socket, &mut b, deadline, &STOP, scan)?;
            raw.push(b[0]);
        }
        raw.truncate(raw.len() - 2);
        Ok(raw)
    }
    fn http_body(
        socket: &UnixStream,
        h: &HttpHead,
        cap: usize,
        deadline: Instant,
        scan: &SharedJournalBudget,
    ) -> Result<Vec<u8>> {
        pay(scan, cap + 1, 1)?;
        let mut raw = Vec::with_capacity(cap + 1);
        if h.chunked {
            loop {
                let size = line(socket, 32, deadline, scan)?;
                let size = std::str::from_utf8(&size).map_err(|_| invalid())?;
                if size.is_empty() || !size.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(invalid());
                }
                let n = usize::from_str_radix(size, 16).map_err(|_| invalid())?;
                if n == 0 {
                    if !line(socket, 2, deadline, scan)?.is_empty() {
                        return Err(invalid());
                    }
                    break;
                }
                if raw.len().checked_add(n).is_none_or(|v| v > cap) {
                    return Err(invalid());
                }
                let start = raw.len();
                raw.resize(start + n, 0);
                root_socket_exact(socket, &mut raw[start..], deadline, &STOP, scan)?;
                let mut end = [0; 2];
                root_socket_exact(socket, &mut end, deadline, &STOP, scan)?;
                if end != *b"\r\n" {
                    return Err(invalid());
                }
            }
        } else if let Some(n) = h.length {
            if n > cap {
                return Err(invalid());
            }
            raw.resize(n, 0);
            root_socket_exact(socket, &mut raw, deadline, &STOP, scan)?;
        } else {
            loop {
                let mut chunk = [0_u8; 8192];
                let available = (cap + 1 - raw.len()).min(chunk.len());
                if available == 0 {
                    return Err(invalid());
                }
                let n = socket_read_some(socket, &mut chunk[..available], deadline, scan)?;
                if n == 0 {
                    return Ok(raw);
                }
                if raw.len() + n > cap {
                    return Err(invalid());
                }
                raw.extend_from_slice(&chunk[..n]);
            }
        }
        let mut extra = [0];
        if socket_read_some(socket, &mut extra, deadline, scan)? != 0 {
            return Err(invalid());
        }
        Ok(raw)
    }
    struct FixedDocker {
        socket_file: File,
        socket_stamp: Stamp,
        daemon_pid: u32,
        daemon_start: u64,
        daemon_pidfd: ParentPid,
        cli: HeldNamed,
        cli_digest: String,
    }
    impl FixedDocker {
        fn open(deadline: Instant, scan: &SharedJournalBudget) -> Result<Self> {
            pay(scan, 128, 3)?;
            let raw = unsafe {
                libc::open(
                    c"/run/docker.sock".as_ptr(),
                    libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if raw < 0 {
                return Err(invalid());
            }
            let socket_file = unsafe { File::from_raw_fd(raw) };
            let socket_stamp = Stamp::observe(&socket_file, scan)?;
            if socket_stamp.uid != 0
                || socket_stamp.mode & libc::S_IFMT != libc::S_IFSOCK
                || socket_stamp.mode & 0o007 != 0
            {
                return Err(invalid());
            }
            let socket = connect_daemon(deadline, scan)?;
            let (daemon_pid, _) = peer(&socket, scan)?;
            let daemon_start = actual_start(daemon_pid, scan)?;
            let daemon_pidfd = ParentPid::open(daemon_pid, daemon_start, scan)?;
            let mut cli = HeldNamed::open("/usr/bin/docker", scan)?;
            let (raw, digest) = cli.full_read_and_hash(64 * 1024 * 1024, scan)?;
            let cli_digest = hex::encode(digest);
            if !raw.starts_with(b"\x7fELF")
                || option_env!("KNOWWEAVE_C4_DOCKER_CLI_SHA256") != Some(cli_digest.as_str())
            {
                return Err(invalid());
            }
            Ok(Self {
                socket_file,
                socket_stamp,
                daemon_pid,
                daemon_start,
                daemon_pidfd,
                cli,
                cli_digest,
            })
        }
        fn socket(&self, deadline: Instant, scan: &SharedJournalBudget) -> Result<UnixStream> {
            self.daemon_pidfd.revalidate(scan)?;
            self.cli.revalidate(scan)?;
            pay(scan, 0, 2)?;
            let m = std::fs::symlink_metadata(DOCKER_SOCKET)?;
            if Stamp::observe(&self.socket_file, scan)? != self.socket_stamp
                || super::Stamp::read(&m).dev != self.socket_stamp.dev
                || m.ino() != self.socket_stamp.ino
                || m.uid() != 0
                || m.mode() & libc::S_IFMT != libc::S_IFSOCK
            {
                return Err(invalid());
            }
            let socket = connect_daemon(deadline, scan)?;
            let (pid, _) = peer(&socket, scan)?;
            if pid != self.daemon_pid || actual_start(pid, scan)? != self.daemon_start {
                return Err(invalid());
            }
            Ok(socket)
        }
        fn request(
            &self,
            method: &str,
            path: &str,
            body: &[u8],
            upgrade: bool,
            deadline: Instant,
            scan: &SharedJournalBudget,
        ) -> Result<(UnixStream, HttpHead)> {
            if !["GET", "POST"].contains(&method)
                || !path.starts_with("/v1.45/")
                || path.contains(['\r', '\n', ' '])
                || body.len() > 16384
            {
                return Err(invalid());
            }
            let socket = self.socket(deadline, scan)?;
            pay(scan, path.len() + body.len() + 1024, 1)?;
            let headers = if upgrade {
                "Connection: Upgrade\r\nUpgrade: tcp\r\n"
            } else {
                "Connection: close\r\n"
            };
            let request = format!(
                "{method} {path} HTTP/1.1\r\nHost: docker\r\n{headers}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            root_socket_all(&socket, request.as_bytes(), deadline, &STOP, scan)?;
            root_socket_all(&socket, body, deadline, &STOP, scan)?;
            let head = http_head(&socket, deadline, scan)?;
            Ok((socket, head))
        }
        fn query(
            &self,
            path: &str,
            cap: usize,
            deadline: Instant,
            scan: &SharedJournalBudget,
        ) -> Result<serde_json::Value> {
            let (socket, head) = self.request("GET", path, &[], false, deadline, scan)?;
            if head.status != 200 {
                return Err(invalid());
            }
            projected_reply(
                path,
                &http_body(&socket, &head, cap, deadline, scan)?,
                cap,
                scan,
            )
        }
        fn create_capture(
            &self,
            catalog: &job::ExecutionCatalog,
            deadline: Instant,
            scan: &SharedJournalBudget,
        ) -> Result<String> {
            #[derive(Serialize)]
            #[serde(rename_all = "PascalCase")]
            struct Create<'a> {
                attach_stdin: bool,
                attach_stdout: bool,
                attach_stderr: bool,
                tty: bool,
                privileged: bool,
                user: &'a str,
                cmd: Vec<&'a str>,
            }
            pay(scan, 4096, 16)?;
            let recipe = &catalog.capture_exec_recipe;
            let cmd = std::iter::once(recipe.entrypoint.as_str())
                .chain(recipe.arguments.iter().map(String::as_str))
                .collect();
            let body = encode(
                &Create {
                    attach_stdin: true,
                    attach_stdout: true,
                    attach_stderr: true,
                    tty: false,
                    privileged: false,
                    user: "0:0",
                    cmd,
                },
                16384,
                scan,
            )?;
            let (socket, h) = self.request(
                "POST",
                &format!(
                    "/v1.45/containers/{}/exec",
                    catalog.installed_runtime.pg_container_id
                ),
                &body,
                false,
                deadline,
                scan,
            )?;
            if h.status != 201 {
                return Err(invalid());
            }
            let raw = http_body(&socket, &h, 4096, deadline, scan)?;
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Created {
                #[serde(rename = "Id")]
                id: String,
            }
            pay(scan, raw.len() * 16, 4)?;
            let created: Created = serde_json::from_slice(&raw)?;
            if !crate::valid_digest(&created.id) {
                return Err(invalid());
            }
            Ok(created.id)
        }
        fn start_capture(
            &self,
            exec: &str,
            deadline: Instant,
            scan: &SharedJournalBudget,
        ) -> Result<UnixStream> {
            if !crate::valid_digest(exec) {
                return Err(invalid());
            }
            let (socket, h) = self.request(
                "POST",
                &format!("/v1.45/exec/{exec}/start"),
                br#"{"Detach":false,"Tty":false}"#,
                true,
                deadline,
                scan,
            )?;
            if h.status != 101 || !h.upgrade || !h.multiplexed || h.length.is_some() || h.chunked {
                return Err(invalid());
            }
            Ok(socket)
        }
        fn inspect_exec(
            &self,
            exec: &str,
            catalog: &job::ExecutionCatalog,
            deadline: Instant,
            scan: &SharedJournalBudget,
        ) -> Result<(bool, Option<i64>, u32)> {
            if !crate::valid_digest(exec) {
                return Err(invalid());
            }
            let v = self.query(&format!("/v1.45/exec/{exec}/json"), 16384, deadline, scan)?;
            let config = &v["ProcessConfig"];
            if jstr(&v, "ID", 64)? != exec
                || jstr(&v, "ContainerID", 64)? != catalog.installed_runtime.pg_container_id
                || !jbool(&v, "OpenStdin")?
                || !jbool(&v, "OpenStdout")?
                || !jbool(&v, "OpenStderr")?
                || jstr(config, "entrypoint", 1024)? != catalog.capture_exec_recipe.entrypoint
                || jbool(config, "tty")?
                || jbool(config, "privileged")?
                || jstr(config, "user", 64)? != "0:0"
            {
                return Err(invalid());
            }
            let args: Vec<String> =
                serde_json::from_value(config.get("arguments").ok_or_else(invalid)?.clone())?;
            if args != catalog.capture_exec_recipe.arguments {
                return Err(invalid());
            }
            let pid = u32::try_from(ju64(&v, "Pid")?).map_err(|_| invalid())?;
            let (running, exit) = exec_exit_data(&v)?;
            Ok((running, exit, pid))
        }
    }

    // One foreground HTTP (or the already-held PG probe) and one fixed
    // all-host reader. Every socket is stored by the original job before its
    // first connect/write. No failed request can be replayed or replaced.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum HttpPhase {
        Connect,
        Write,
        Head,
        Body,
        ChunkLine,
        ChunkData,
        ChunkCrLf,
        ChunkEnd,
        Tail,
        Upgrade,
        Done,
    }
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum HttpPurpose {
        Positive,
        HostObserver,
        NegativeExactExecInspect,
    }

    // A cost receipt bound to the exact original owned socket, never a role or
    // authority permit. States are available/claimed/closed/unknown. Claim is
    // before syscall; EINTR/ENOTCONN cannot mint retry credit or another slot.
    struct PrepaidShutdown {
        socket: Arc<UnixStream>,
        state: Arc<std::sync::atomic::AtomicU8>,
    }
    impl PrepaidShutdown {
        fn bind(socket: Arc<UnixStream>, state: Arc<std::sync::atomic::AtomicU8>) -> Self {
            Self { socket, state }
        }
        fn close_once(&self) -> Result<bool> {
            if self
                .state
                .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return Ok(false);
            }
            let result = unsafe { libc::shutdown(self.socket.as_raw_fd(), libc::SHUT_WR) };
            self.state
                .store(if result == 0 { 2 } else { 3 }, Ordering::Release);
            if result == 0 {
                Ok(true)
            } else {
                Err(invalid())
            }
        }
    }

    struct DockerBuild {
        socket_file: Option<File>,
        socket_stamp: Option<Stamp>,
        cli: Option<HeldNamed>,
        cli_digest: Option<String>,
        daemon_pidfd: Option<ParentPid>,
        daemon_pid: u32,
        daemon_start: u64,
    }
    struct OriginalHttpOwner {
        socket: Arc<UnixStream>,
        shutdown: PrepaidShutdown,
        purpose: HttpPurpose,
        negative_eof: bool,
        negative_drained: usize,
        path: String,
        request: Vec<u8>,
        written: usize,
        header: Vec<u8>,
        head: Option<HttpHead>,
        body: Vec<u8>,
        line: Vec<u8>,
        phase: HttpPhase,
        remaining: usize,
        delimiter: usize,
        cap: usize,
        deadline: Instant,
        connect_started: bool,
        peer_checked: bool,
        upgrade: bool,
    }
    impl OriginalHttpOwner {
        // Creation has no connect or external operation. The next call to
        // advance is only possible after this exact owner enters its Book slot.
        fn new(
            request: (&str, &str, &[u8]),
            upgrade: bool,
            cap: usize,
            deadline: Instant,
            scan: &SharedJournalBudget,
            purpose: HttpPurpose,
        ) -> Result<Self> {
            let (method, path, body) = request;
            if !["GET", "POST"].contains(&method)
                || !path.starts_with("/v1.45/")
                || path.contains(['\r', '\n', ' '])
                || body.len() > 16384
                || cap > RECORD_CAP
            {
                return Err(invalid());
            }
            pay(scan, path.len() * 2 + body.len() * 2 + 8192 + 256, 8)?;
            let headers = if upgrade {
                "Connection: Upgrade\r\nUpgrade: tcp\r\n"
            } else {
                "Connection: close\r\n"
            };
            let mut request=format!("{method} {path} HTTP/1.1\r\nHost: docker\r\n{headers}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n",body.len()).into_bytes();
            request.extend_from_slice(body);
            let header = Vec::with_capacity(8192);
            let result = Vec::new();
            let line = Vec::with_capacity(34);
            let path = path.to_owned();
            // This reader has its own additional negative close cost; it is
            // not one of the four core Source/mux closures.
            pay(scan, 128, 1)?;
            let close_state = Arc::new(std::sync::atomic::AtomicU8::new(0));
            fd_gate(2, scan)?;
            pay(scan, 0, 1)?;
            let raw = unsafe {
                libc::socket(
                    libc::AF_UNIX,
                    libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                    0,
                )
            };
            if raw < 0 {
                return Err(invalid());
            }
            let socket = Arc::new(unsafe { UnixStream::from_raw_fd(raw) });
            let shutdown = PrepaidShutdown::bind(Arc::clone(&socket), close_state);
            Ok(Self {
                socket,
                shutdown,
                purpose,
                negative_eof: false,
                negative_drained: 0,
                path,
                request,
                written: 0,
                header,
                head: None,
                body: result,
                line,
                phase: HttpPhase::Connect,
                remaining: 0,
                delimiter: 0,
                cap,
                deadline,
                connect_started: false,
                peer_checked: false,
                upgrade,
            })
        }
        fn events(&self) -> i16 {
            match self.phase {
                HttpPhase::Connect | HttpPhase::Write => libc::POLLOUT,
                HttpPhase::Done | HttpPhase::Upgrade => 0,
                _ => libc::POLLIN,
            }
        }
        fn boundary(&mut self, deadline: Instant) -> Result<()> {
            self.deadline = self.deadline.min(deadline);
            root_io_boundary(self.deadline, &STOP)
        }
        fn read(&self, out: &mut [u8], scan: &SharedJournalBudget) -> Result<Option<usize>> {
            pay(scan, out.len(), 1)?;
            let n = unsafe {
                libc::recv(
                    self.socket.as_raw_fd(),
                    out.as_mut_ptr().cast(),
                    out.len(),
                    libc::MSG_DONTWAIT,
                )
            };
            if n >= 0 {
                return Ok(Some(n as usize));
            }
            if matches!(
                std::io::Error::last_os_error().kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
            ) {
                Ok(None)
            } else {
                Err(invalid())
            }
        }
        // Exactly one bounded syscall/parse transition per tick. A new chunk,
        // line, EOF probe or retry never borrows an uncharged allocation.
        fn advance(
            &mut self,
            daemon: (u32, u64),
            deadline: Instant,
            scan: &SharedJournalBudget,
        ) -> Result<()> {
            self.boundary(deadline)?;
            match self.phase {
                HttpPhase::Connect => {
                    if !self.connect_started {
                        pay(scan, 256, 1)?;
                        let mut address = unsafe { std::mem::zeroed::<libc::sockaddr_un>() };
                        address.sun_family = libc::AF_UNIX as libc::sa_family_t;
                        for (to, from) in address.sun_path.iter_mut().zip(DOCKER_SOCKET.as_bytes())
                        {
                            *to = *from as libc::c_char;
                        }
                        self.connect_started = true;
                        let n = unsafe {
                            libc::connect(
                                self.socket.as_raw_fd(),
                                (&address as *const libc::sockaddr_un).cast(),
                                std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
                            )
                        };
                        if n != 0
                            && !matches!(
                                std::io::Error::last_os_error().raw_os_error(),
                                Some(libc::EINPROGRESS) | Some(libc::EAGAIN) | Some(libc::EINTR)
                            )
                        {
                            return Err(invalid());
                        }
                        if n != 0 {
                            return Ok(());
                        }
                    } else {
                        pay(scan, 0, 1)?;
                        let mut p = libc::pollfd {
                            fd: self.socket.as_raw_fd(),
                            events: libc::POLLOUT,
                            revents: 0,
                        };
                        let n = unsafe { libc::poll(&mut p, 1, 0) };
                        if n < 0 {
                            return if std::io::Error::last_os_error().kind()
                                == std::io::ErrorKind::Interrupted
                            {
                                Ok(())
                            } else {
                                Err(invalid())
                            };
                        }
                        if n == 0 {
                            return Ok(());
                        }
                        if p.revents & (libc::POLLERR | libc::POLLNVAL | libc::POLLHUP) != 0 {
                            return Err(invalid());
                        }
                    }
                    let mut error = 0_i32;
                    let mut size = std::mem::size_of_val(&error) as libc::socklen_t;
                    pay(scan, 0, 1)?;
                    if unsafe {
                        libc::getsockopt(
                            self.socket.as_raw_fd(),
                            libc::SOL_SOCKET,
                            libc::SO_ERROR,
                            (&mut error as *mut i32).cast(),
                            &mut size,
                        )
                    } != 0
                        || error != 0
                    {
                        return Err(invalid());
                    }
                    let (pid, _) = peer(&self.socket, scan)?;
                    if pid != daemon.0 || actual_start(pid, scan)? != daemon.1 {
                        return Err(invalid());
                    }
                    self.peer_checked = true;
                    self.phase = HttpPhase::Write;
                }
                HttpPhase::Write => {
                    if !self.peer_checked {
                        return Err(invalid());
                    }
                    let end = (self.written + 8192).min(self.request.len());
                    let bytes = &self.request[self.written..end];
                    pay(scan, bytes.len(), 1)?;
                    let n = unsafe {
                        libc::send(
                            self.socket.as_raw_fd(),
                            bytes.as_ptr().cast(),
                            bytes.len(),
                            libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
                        )
                    };
                    if n > 0 {
                        self.written += n as usize;
                        if self.written == self.request.len() {
                            self.phase = HttpPhase::Head;
                        }
                    } else if n == 0
                        || !matches!(
                            std::io::Error::last_os_error().kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                        )
                    {
                        return Err(invalid());
                    }
                }
                HttpPhase::Head => {
                    let mut byte = [0];
                    if let Some(n) = self.read(&mut byte, scan)? {
                        if n == 0 {
                            return Err(invalid());
                        }
                        if self.header.len() == 8192 {
                            return Err(invalid());
                        }
                        self.header.push(byte[0]);
                        if self.header.ends_with(b"\r\n\r\n") {
                            let head = parse_original_http_head(&self.header, scan)?;
                            self.phase = if self.upgrade {
                                if head.status != 101
                                    || !head.upgrade
                                    || !head.multiplexed
                                    || head.length.is_some()
                                    || head.chunked
                                {
                                    return Err(invalid());
                                }
                                HttpPhase::Upgrade
                            } else if head.chunked {
                                HttpPhase::ChunkLine
                            } else if let Some(length) = head.length {
                                if length > self.cap {
                                    return Err(invalid());
                                }
                                self.remaining = length;
                                if length == 0 {
                                    HttpPhase::Tail
                                } else {
                                    HttpPhase::Body
                                }
                            } else {
                                HttpPhase::Body
                            };
                            self.head = Some(head);
                        }
                    }
                }
                HttpPhase::Body | HttpPhase::ChunkData => {
                    let mut raw = [0_u8; 8192];
                    let fixed = self.phase == HttpPhase::ChunkData
                        || self.head.as_ref().is_some_and(|h| h.length.is_some());
                    let want = if fixed {
                        self.remaining.min(raw.len())
                    } else {
                        (self.cap + 1 - self.body.len()).min(raw.len())
                    };
                    if want == 0 {
                        return Err(invalid());
                    }
                    if let Some(n) = self.read(&mut raw[..want], scan)? {
                        if n == 0 {
                            if fixed {
                                return Err(invalid());
                            }
                            self.phase = HttpPhase::Done;
                        } else {
                            if self.body.len().checked_add(n).is_none_or(|n| n > self.cap) {
                                return Err(invalid());
                            }
                            pay(scan, n * 2 + 64, 1)?;
                            self.body.try_reserve_exact(n).map_err(|_| invalid())?;
                            self.body.extend_from_slice(&raw[..n]);
                            if fixed {
                                self.remaining -= n;
                                if self.remaining == 0 {
                                    self.phase = if self.phase == HttpPhase::ChunkData {
                                        HttpPhase::ChunkCrLf
                                    } else {
                                        HttpPhase::Tail
                                    };
                                }
                            }
                        }
                    }
                }
                HttpPhase::ChunkLine | HttpPhase::ChunkEnd => {
                    let mut byte = [0];
                    if let Some(n) = self.read(&mut byte, scan)? {
                        if n == 0 {
                            return Err(invalid());
                        }
                        if self.line.len() >= 34 {
                            return Err(invalid());
                        }
                        self.line.push(byte[0]);
                        if self.line.ends_with(b"\r\n") {
                            pay(scan, self.line.len() * 8 + 64, 4)?;
                            if self.phase == HttpPhase::ChunkEnd {
                                if self.line != b"\r\n" {
                                    return Err(invalid());
                                }
                                self.line.clear();
                                self.phase = HttpPhase::Tail;
                            } else {
                                let size = &self.line[..self.line.len() - 2];
                                if size.is_empty()
                                    || size.len() > 32
                                    || !size.iter().all(u8::is_ascii_hexdigit)
                                {
                                    return Err(invalid());
                                }
                                let count = usize::from_str_radix(
                                    std::str::from_utf8(size).map_err(|_| invalid())?,
                                    16,
                                )
                                .map_err(|_| invalid())?;
                                self.line.clear();
                                if self
                                    .body
                                    .len()
                                    .checked_add(count)
                                    .is_none_or(|n| n > self.cap)
                                {
                                    return Err(invalid());
                                }
                                self.remaining = count;
                                self.phase = if count == 0 {
                                    HttpPhase::ChunkEnd
                                } else {
                                    HttpPhase::ChunkData
                                };
                            }
                        }
                    }
                }
                HttpPhase::ChunkCrLf => {
                    let mut byte = [0];
                    if let Some(n) = self.read(&mut byte, scan)? {
                        if n == 0 || byte[0] != b"\r\n"[self.delimiter] {
                            return Err(invalid());
                        }
                        self.delimiter += 1;
                        if self.delimiter == 2 {
                            self.delimiter = 0;
                            self.phase = HttpPhase::ChunkLine;
                        }
                    }
                }
                HttpPhase::Tail => {
                    let mut byte = [0];
                    if let Some(n) = self.read(&mut byte, scan)? {
                        if n != 0 {
                            return Err(invalid());
                        }
                        self.phase = HttpPhase::Done;
                    }
                }
                HttpPhase::Upgrade | HttpPhase::Done => {}
            }
            self.boundary(deadline)
        }
    }

    fn negative_http_drain(
        owner: &mut OriginalHttpOwner,
        deadline: Instant,
        scan: &SharedJournalBudget,
    ) -> Result<()> {
        // An exact negative inspection is a new bounded read-only observation
        // owned by its own parser. Only an old failed request is raw-drained.
        if owner.purpose == HttpPurpose::NegativeExactExecInspect || owner.negative_eof {
            return Ok(());
        }
        root_io_boundary(deadline, &STOP)?;
        let mut bytes = [0_u8; 256];
        pay(scan, bytes.len(), 1)?;
        let n = unsafe {
            libc::recv(
                owner.socket.as_raw_fd(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
                libc::MSG_DONTWAIT,
            )
        };
        if n == 0 {
            owner.negative_eof = true;
        } else if n > 0 {
            owner.negative_drained = owner
                .negative_drained
                .checked_add(n as usize)
                .filter(|n| *n <= owner.cap + 8192)
                .ok_or_else(invalid)?;
        } else if !matches!(
            std::io::Error::last_os_error().kind(),
            std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
        ) && std::io::Error::last_os_error().raw_os_error() != Some(libc::ENOTCONN)
        {
            return Err(invalid());
        }
        root_io_boundary(deadline, &STOP)
    }

    fn parse_original_http_head(raw: &[u8], scan: &SharedJournalBudget) -> Result<HttpHead> {
        if raw.len() > 8192 || !raw.ends_with(b"\r\n\r\n") {
            return Err(invalid());
        }
        pay(scan, raw.len() * 8 + 1024, raw.len() + 64)?;
        let text = std::str::from_utf8(raw).map_err(|_| invalid())?;
        let mut lines = text.split("\r\n");
        let status_line = lines.next().ok_or_else(invalid)?;
        let mut words = status_line.splitn(3, ' ');
        let protocol = words.next().ok_or_else(invalid)?;
        let code = words.next().ok_or_else(invalid)?;
        let _reason = words.next().ok_or_else(invalid)?;
        if protocol != "HTTP/1.1" {
            return Err(invalid());
        }
        let status = code.parse::<u16>().map_err(|_| invalid())?;
        let (mut length, mut chunked, mut multiplexed, mut upgrade) = (None, false, false, false);
        let mut names = BTreeSet::new();
        for line in lines {
            if line.is_empty() {
                continue;
            }
            let (name, value) = line.split_once(':').ok_or_else(invalid)?;
            let name = name.to_ascii_lowercase();
            if !names.insert(name.clone()) {
                return Err(invalid());
            }
            let value = value.trim();
            match name.as_str() {
                "content-length" => length = Some(value.parse::<usize>().map_err(|_| invalid())?),
                "transfer-encoding" => {
                    if value != "chunked" {
                        return Err(invalid());
                    }
                    chunked = true;
                }
                "content-type" => {
                    multiplexed = value == "application/vnd.docker.multiplexed-stream"
                }
                "upgrade" => upgrade = value.eq_ignore_ascii_case("tcp"),
                _ => {}
            }
        }
        if length.is_some() && chunked {
            return Err(invalid());
        }
        Ok(HttpHead {
            status,
            length,
            chunked,
            multiplexed,
            upgrade,
        })
    }
    struct FixedHostPack {
        info: Option<serde_json::Value>,
        listed: Option<serde_json::Value>,
        ids: Vec<String>,
        containers: BTreeMap<String, serde_json::Value>,
        network: Option<serde_json::Value>,
        image: Option<serde_json::Value>,
    }
    struct FixedHostScan {
        http: Option<OriginalHttpOwner>,
        pack: FixedHostPack,
        index: usize,
        checked_index: usize,
        ids_checked: bool,
        network_checked: bool,
        image_checked: bool,
        stage: u8,
        deadline: Instant,
        started: Instant,
    }
    impl FixedHostScan {
        fn new(deadline: Instant, scan: &SharedJournalBudget) -> Result<Self> {
            pay(scan, 4096, 8)?;
            Ok(Self {
                http: None,
                pack: FixedHostPack {
                    info: None,
                    listed: None,
                    ids: Vec::new(),
                    containers: BTreeMap::new(),
                    network: None,
                    image: None,
                },
                index: 0,
                checked_index: 0,
                ids_checked: false,
                network_checked: false,
                image_checked: false,
                stage: 0,
                deadline,
                started: Instant::now(),
            })
        }
        fn next_path(
            &self,
            catalog: &job::ExecutionCatalog,
            scan: &SharedJournalBudget,
        ) -> Result<String> {
            pay(scan, 512, 1)?;
            Ok(match self.stage {
                0 => "/v1.45/info".into(),
                1 => "/v1.45/containers/json?all=1&size=0".into(),
                2 => format!(
                    "/v1.45/containers/{}/json",
                    self.pack.ids.get(self.index).ok_or_else(invalid)?
                ),
                3 => format!(
                    "/v1.45/networks/{}",
                    catalog.installed_runtime.internal_network_id
                ),
                4 => format!(
                    "/v1.45/images/{}/json",
                    catalog.installed_runtime.admitted_pg_image_id
                ),
                _ => return Err(invalid()),
            })
        }
        fn advance(
            &mut self,
            catalog: &job::ExecutionCatalog,
            daemon: (u32, u64),
            deadline: Instant,
            scan: &SharedJournalBudget,
        ) -> Result<bool> {
            self.deadline = self.deadline.min(deadline);
            root_io_boundary(self.deadline, &STOP)?;
            if self.stage == 5 {
                return Ok(true);
            }
            if self.http.is_none() {
                let path = self.next_path(catalog, scan)?;
                self.http = Some(OriginalHttpOwner::new(
                    ("GET", &path, &[]),
                    false,
                    RECORD_CAP,
                    self.deadline,
                    scan,
                    HttpPurpose::HostObserver,
                )?);
                return Ok(false);
            }
            let current = self.http.as_mut().ok_or_else(invalid)?;
            current.advance(daemon, self.deadline, scan)?;
            if current.phase != HttpPhase::Done {
                return Ok(false);
            }
            if current.head.as_ref().ok_or_else(invalid)?.status != 200 {
                return Err(invalid());
            }
            let value = projected_reply(&current.path, &current.body, RECORD_CAP, scan)?;
            match self.stage {
                0 => {
                    if jstr(&value, "ID", 128)? != catalog.installed_runtime.daemon_id {
                        return Err(invalid());
                    }
                    self.pack.info = Some(value);
                    self.stage = 1;
                }
                1 => {
                    let rows = value.as_array().ok_or_else(invalid)?;
                    if rows.is_empty() || rows.len() > CONTAINER_CAP {
                        return Err(invalid());
                    }
                    pay(scan, rows.len() * 128, rows.len() * 2)?;
                    let mut ids = rows
                        .iter()
                        .map(|r| jstr(r, "Id", 64).map(str::to_owned))
                        .collect::<Result<Vec<_>>>()?;
                    ids.sort();
                    if ids.iter().any(|v| !crate::valid_digest(v))
                        || ids.windows(2).any(|v| v[0] == v[1])
                    {
                        return Err(invalid());
                    }
                    self.pack.ids = ids;
                    self.pack.listed = Some(value);
                    self.stage = 2;
                }
                2 => {
                    let id = self.pack.ids.get(self.index).ok_or_else(invalid)?;
                    if jstr(&value, "Id", 64)? != id {
                        return Err(invalid());
                    }
                    pay(scan, id.len() + 256, 1)?;
                    self.pack.containers.insert(id.clone(), value);
                    self.index += 1;
                    if self.index == self.pack.ids.len() {
                        self.stage = 3;
                    }
                }
                3 => {
                    self.pack.network = Some(value);
                    self.stage = 4;
                }
                4 => {
                    self.pack.image = Some(value);
                    self.stage = 5;
                }
                _ => return Err(invalid()),
            }
            // Only a read-only request with actual physical EOF may end this
            // socket duty. An error before here keeps the same owner in place.
            self.http = None;
            root_io_boundary(self.deadline, &STOP)?;
            Ok(self.stage == 5)
        }
    }

    struct AttachedResult {
        stdout: [u8; 32],
        stderr: [u8; 32],
        double_eof: bool,
        no_extra: bool,
        stdin_bytes: usize,
    }
    struct AttachedOwner {
        control: Arc<UnixStream>,
        input_shutdown: Arc<PrepaidShutdown>,
        output_shutdown: Arc<PrepaidShutdown>,
        physical_shutdown: Arc<PrepaidShutdown>,
        transport: Arc<UnixStream>,
        bridge: Arc<UnixStream>,
        expected_stdin: usize,
        worker: Option<JoinHandle<Result<AttachedResult>>>,
        result: Option<AttachedResult>,
        failed: bool,
    }
    impl AttachedOwner {
        fn start(
            attached: Arc<UnixStream>,
            physical_shutdown: Arc<PrepaidShutdown>,
            input_slot: Arc<std::sync::atomic::AtomicU8>,
            output_slot: Arc<std::sync::atomic::AtomicU8>,
            failed: Arc<AtomicBool>,
            total: Instant,
            scan: &SharedJournalBudget,
        ) -> Result<Self> {
            if !Arc::ptr_eq(&attached, &physical_shutdown.socket) {
                return Err(invalid());
            }
            fd_gate(4, scan)?;
            pay(scan, CONTROL_BYTES as usize * 2 + 65536, 8)?;
            let (control, bridge) = UnixStream::pair()?;
            mark_cloexec_nonblocking(control.as_raw_fd())?;
            mark_cloexec_nonblocking(bridge.as_raw_fd())?;
            let control = Arc::new(control);
            let bridge = Arc::new(bridge);
            let input_shutdown = Arc::new(PrepaidShutdown::bind(Arc::clone(&control), input_slot));
            let output_shutdown = Arc::new(PrepaidShutdown::bind(Arc::clone(&bridge), output_slot));
            let transport = Arc::clone(&attached);
            let bridge_owner = Arc::clone(&bridge);
            let meter = scan.clone();
            let physical_close = Arc::clone(&physical_shutdown);
            let output_close = Arc::clone(&output_shutdown);
            let worker = std::thread::Builder::new()
                .name("b1-fixed-docker-mux".into())
                .spawn(move || {
                    mux(
                        attached,
                        bridge,
                        total,
                        meter,
                        physical_close,
                        output_close,
                        failed,
                    )
                })?;
            Ok(Self {
                control,
                input_shutdown,
                output_shutdown,
                physical_shutdown,
                transport,
                bridge: bridge_owner,
                expected_stdin: 0,
                worker: Some(worker),
                result: None,
                failed: false,
            })
        }
        fn joined(&mut self) -> Result<bool> {
            if self.failed {
                return Err(invalid());
            }
            if self.result.is_some() {
                return Ok(true);
            }
            if self.worker.as_ref().is_none_or(|w| !w.is_finished()) {
                return Ok(false);
            }
            self.failed = true;
            let value = self
                .worker
                .take()
                .ok_or_else(invalid)?
                .join()
                .map_err(|_| invalid())??;
            if !value.double_eof || !value.no_extra || value.stdin_bytes != self.expected_stdin {
                return Err(invalid());
            }
            self.result = Some(value);
            self.failed = false;
            Ok(true)
        }
    }
    fn mux(
        attached: Arc<UnixStream>,
        bridge: Arc<UnixStream>,
        total: Instant,
        scan: SharedJournalBudget,
        physical_close: Arc<PrepaidShutdown>,
        output_close: Arc<PrepaidShutdown>,
        failed: Arc<AtomicBool>,
    ) -> Result<AttachedResult> {
        let mut output = VecDeque::<u8>::with_capacity(CONTROL_BYTES as usize);
        let mut stdin = VecDeque::<u8>::with_capacity(CONTROL_BYTES as usize);
        let (mut stdout_hash, mut stderr_hash) = (Sha256::new(), Sha256::new());
        let (mut stdout_bytes, mut stderr_bytes, mut stdin_bytes) = (0_usize, 0_usize, 0_usize);
        let mut written_stdin = 0_usize;
        let mut header = [0_u8; 8];
        let mut header_used = 0_usize;
        let mut remaining = 0_usize;
        let mut stream = 0_u8;
        let mut eof = false;
        let mut input_eof = false;
        loop {
            if failed.load(Ordering::Acquire) {
                return Err(invalid());
            }
            root_io_boundary(total, &STOP)?;
            if input_eof && stdin.is_empty() {
                physical_close.close_once()?;
            }
            if eof && output.is_empty() && input_eof && stdin.is_empty() {
                output_close.close_once()?;
                break;
            }
            let mut poll = [
                libc::pollfd {
                    fd: attached.as_raw_fd(),
                    events: if eof { 0 } else { libc::POLLIN }
                        | if stdin.is_empty() { 0 } else { libc::POLLOUT },
                    revents: 0,
                },
                libc::pollfd {
                    fd: bridge.as_raw_fd(),
                    events: if input_eof { 0 } else { libc::POLLIN }
                        | if output.is_empty() { 0 } else { libc::POLLOUT },
                    revents: 0,
                },
            ];
            pay(&scan, 0, 1)?;
            let millis = total
                .saturating_duration_since(Instant::now())
                .as_millis()
                .min(100) as i32;
            let n = unsafe { libc::poll(poll.as_mut_ptr(), 2, millis) };
            if n < 0 {
                if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(invalid());
            }
            if poll
                .iter()
                .any(|p| p.revents & (libc::POLLNVAL | libc::POLLERR) != 0)
            {
                return Err(invalid());
            }
            if poll[1].revents & libc::POLLOUT != 0 && !output.is_empty() {
                if failed.load(Ordering::Acquire) {
                    return Err(invalid());
                }
                let part = output.as_slices().0;
                pay(&scan, part.len(), 1)?;
                let n = unsafe {
                    libc::send(
                        bridge.as_raw_fd(),
                        part.as_ptr().cast(),
                        part.len(),
                        libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
                    )
                };
                if n > 0 {
                    output.drain(..n as usize);
                } else if n == 0
                    || !matches!(
                        std::io::Error::last_os_error().kind(),
                        std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                    )
                {
                    return Err(invalid());
                }
            }
            if poll[0].revents & libc::POLLOUT != 0 && !stdin.is_empty() {
                if failed.load(Ordering::Acquire) {
                    return Err(invalid());
                }
                let part = stdin.as_slices().0;
                pay(&scan, part.len(), 1)?;
                let n = unsafe {
                    libc::send(
                        attached.as_raw_fd(),
                        part.as_ptr().cast(),
                        part.len(),
                        libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
                    )
                };
                if n > 0 {
                    written_stdin = written_stdin.checked_add(n as usize).ok_or_else(invalid)?;
                    stdin.drain(..n as usize);
                } else if n == 0
                    || !matches!(
                        std::io::Error::last_os_error().kind(),
                        std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                    )
                {
                    return Err(invalid());
                }
            }
            if poll[1].revents & (libc::POLLIN | libc::POLLHUP) != 0 && !input_eof {
                let mut b = [0_u8; 8192];
                pay(&scan, b.len(), 1)?;
                let n = unsafe {
                    libc::recv(
                        bridge.as_raw_fd(),
                        b.as_mut_ptr().cast(),
                        b.len(),
                        libc::MSG_DONTWAIT,
                    )
                };
                if n == 0 {
                    input_eof = true;
                } else if n > 0 {
                    let n = n as usize;
                    stdin_bytes = stdin_bytes
                        .checked_add(n)
                        .filter(|x| *x <= CONTROL_BYTES as usize)
                        .ok_or_else(invalid)?;
                    stdin.extend(&b[..n]);
                } else if !matches!(
                    std::io::Error::last_os_error().kind(),
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                ) {
                    return Err(invalid());
                }
            }
            if poll[0].revents & (libc::POLLIN | libc::POLLHUP) != 0 && !eof {
                if remaining == 0 && header_used < 8 {
                    pay(&scan, 8 - header_used, 1)?;
                    let n = unsafe {
                        libc::recv(
                            attached.as_raw_fd(),
                            header[header_used..].as_mut_ptr().cast(),
                            8 - header_used,
                            libc::MSG_DONTWAIT,
                        )
                    };
                    if n == 0 {
                        if header_used != 0 || !stdin.is_empty() {
                            return Err(invalid());
                        }
                        eof = true;
                        continue;
                    }
                    if n < 0 {
                        if matches!(
                            std::io::Error::last_os_error().kind(),
                            std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                        ) {
                            continue;
                        }
                        return Err(invalid());
                    }
                    header_used += n as usize;
                    if header_used != 8 {
                        continue;
                    }
                    if !matches!(header[0], 1 | 2) || header[1..4] != [0, 0, 0] {
                        return Err(invalid());
                    }
                    stream = header[0];
                    remaining = u32::from_be_bytes(header[4..8].try_into().map_err(|_| invalid())?)
                        as usize;
                    if remaining == 0 || remaining > 1024 * 1024 {
                        return Err(invalid());
                    }
                    let old = if stream == 1 {
                        stdout_bytes
                    } else {
                        stderr_bytes
                    };
                    let cap = if stream == 1 {
                        CONTROL_BYTES as usize
                    } else {
                        1024 * 1024
                    };
                    if old.checked_add(remaining).is_none_or(|n| n > cap) {
                        return Err(invalid());
                    }
                }
                if remaining > 0 {
                    let mut b = [0_u8; 8192];
                    let want = remaining.min(b.len());
                    pay(&scan, want * 2, 1)?;
                    let n = unsafe {
                        libc::recv(
                            attached.as_raw_fd(),
                            b.as_mut_ptr().cast(),
                            want,
                            libc::MSG_DONTWAIT,
                        )
                    };
                    if n == 0 {
                        return Err(invalid());
                    }
                    if n < 0 {
                        if matches!(
                            std::io::Error::last_os_error().kind(),
                            std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                        ) {
                            continue;
                        }
                        return Err(invalid());
                    }
                    let n = n as usize;
                    if stream == 1 {
                        stdout_hash.update(&b[..n]);
                        stdout_bytes += n;
                        output.extend(&b[..n]);
                    } else {
                        stderr_hash.update(&b[..n]);
                        stderr_bytes += n;
                    }
                    remaining -= n;
                    if remaining == 0 {
                        header_used = 0;
                    }
                }
            }
        }
        Ok(AttachedResult {
            stdout: stdout_hash.finalize().into(),
            stderr: stderr_hash.finalize().into(),
            double_eof: eof && header_used == 0 && remaining == 0,
            no_extra: stdin.is_empty() && input_eof,
            stdin_bytes: written_stdin,
        })
    }
    const TOPOLOGY: [(&str, &str, bool); 12] = [
        ("/opt/knowweave-c4/b1-capture", "bind", false),
        ("/run/kwc4/sfx", "bind", true),
        ("/usr/bin/docker", "bind", false),
        (OBSERVATION_ROOT, "bind", false),
        ("/var/lib/knowweave-c4/registry", "volume", true),
        (InstalledSnapshotBinding::COMPANION_PATH, "volume", true),
        (InstalledSnapshotBinding::EXPORT_PATH, "volume", true),
        ("/var/lib/knowweave-source", "volume", true),
        ("/var/lib/knowweave-source/assets", "bind", false),
        ("/var/lib/postgresql", "volume", true),
        ("/var/run/docker.sock", "bind", false),
        ("/var/run/postgresql", "volume", true),
    ];
    fn projection(
        v: &serde_json::Value,
        scan: &SharedJournalBudget,
    ) -> Result<ContainerProjection> {
        let mounts = v["Mounts"].as_array().ok_or_else(invalid)?;
        if mounts.len() > 1024 {
            return Err(invalid());
        }
        pay(scan, mounts.len() * 4096 + 2048, mounts.len() + 1)?;
        let mut rows = Vec::with_capacity(mounts.len());
        for raw in mounts {
            let m: ActualAssetsMount = serde_json::from_value(raw.clone())?;
            let row = MountProjection {
                source: m.source,
                destination: m.destination,
                rw: m.rw,
                kind: m.kind,
                name: m.name.normalized().map(str::to_owned),
            };
            row.validate()?;
            rows.push(row);
        }
        rows.sort_by(|a, b| a.destination.cmp(&b.destination));
        let label = |key: &str| -> Result<Option<String>> {
            match &v["Config"]["Labels"][key] {
                serde_json::Value::Null => Ok(None),
                serde_json::Value::String(s) if text(s, 1024) => Ok(Some(s.clone())),
                _ => Err(invalid()),
            }
        };
        let row = ContainerProjection {
            container_id: jstr(v, "Id", 64)?.into(),
            image_sha256: jstr(v, "Image", 71)?.into(),
            running: jbool(&v["State"], "Running")?,
            process_id: ju64(&v["State"], "Pid")?,
            started_at: jstr(&v["State"], "StartedAt", 128)?.into(),
            restart_count: ju64(v, "RestartCount")?,
            project: label("com.docker.compose.project")?,
            service: label("com.docker.compose.service")?,
            mounts: rows,
        };
        row.validate()?;
        Ok(row)
    }
    fn mapped(rows: &[MountProjection], inside: &str) -> Result<String> {
        let m = rows
            .iter()
            .filter(|m| contains(&m.destination, inside))
            .max_by_key(|m| m.destination.len())
            .ok_or_else(invalid)?;
        let value = format!(
            "{}{}",
            m.source,
            inside.strip_prefix(&m.destination).ok_or_else(invalid)?
        );
        if !path(&value) {
            return Err(invalid());
        }
        Ok(value)
    }
    fn clone_data<T: Serialize + DeserializeOwned>(
        data: &T,
        cap: usize,
        scan: &SharedJournalBudget,
    ) -> Result<T> {
        parse(&encode(data, cap, scan)?, cap, scan)
    }
    struct HostObservation {
        data: HostCutoverData,
        network: Vec<u8>,
        image: Vec<u8>,
        security: Vec<u8>,
        pg_pid: u32,
        current_mounts: Vec<MountProjection>,
    }
    impl FixedDocker {
        fn observe_host(
            &self,
            catalog: &job::ExecutionCatalog,
            deadline: Instant,
            proc: &ProcView,
            scan: &SharedJournalBudget,
        ) -> Result<HostObservation> {
            let r = &catalog.installed_runtime;
            let info = self.query("/v1.45/info", RECORD_CAP, deadline, scan)?;
            if jstr(&info, "ID", 128)? != r.daemon_id {
                return Err(invalid());
            }
            let listed = self.query(
                "/v1.45/containers/json?all=1&size=0",
                RECORD_CAP,
                deadline,
                scan,
            )?;
            let all = listed.as_array().ok_or_else(invalid)?;
            if all.is_empty() || all.len() > CONTAINER_CAP {
                return Err(invalid());
            }
            pay(scan, all.len() * 128, all.len())?;
            let mut ids = all
                .iter()
                .map(|x| jstr(x, "Id", 64).map(str::to_owned))
                .collect::<Result<Vec<_>>>()?;
            ids.sort();
            if ids.iter().any(|x| !crate::valid_digest(x)) || ids.windows(2).any(|x| x[0] == x[1]) {
                return Err(invalid());
            }
            let mut rows = Vec::with_capacity(ids.len());
            let mut own = None;
            for id in ids {
                root_io_boundary(deadline, &STOP)?;
                let raw = self.query(
                    &format!("/v1.45/containers/{id}/json"),
                    RECORD_CAP,
                    deadline,
                    scan,
                )?;
                let row = projection(&raw, scan)?;
                if row.container_id != id {
                    return Err(invalid());
                }
                if id == r.pg_container_id {
                    own = Some(raw);
                }
                rows.push(row);
            }
            let own = own.ok_or_else(invalid)?;
            let current = rows
                .iter()
                .find(|c| c.container_id == r.pg_container_id)
                .ok_or_else(invalid)?;
            if !current.running
                || current.image_sha256 != r.admitted_pg_image_id
                || current.project.as_deref() != Some(&r.compose_project)
                || current.service.as_deref() != Some("pg")
                || current.mounts.len() != 12
                || jstr(&own["Config"], "Image", 256)? != PG_IMAGE
            {
                return Err(invalid());
            }
            for (m, (destination, kind, rw)) in current.mounts.iter().zip(TOPOLOGY) {
                if m.destination != destination
                    || m.kind != kind
                    || m.rw != rw
                    || (kind == "volume" && m.name.as_ref().is_none_or(String::is_empty))
                {
                    return Err(invalid());
                }
            }
            let networks = own["NetworkSettings"]["Networks"]
                .as_object()
                .ok_or_else(invalid)?;
            if networks.len() != 1 {
                return Err(invalid());
            }
            let (network_name, attachment) = networks.iter().next().ok_or_else(invalid)?;
            if jstr(attachment, "NetworkID", 64)? != r.internal_network_id {
                return Err(invalid());
            }
            let network = self.query(
                &format!("/v1.45/networks/{}", r.internal_network_id),
                RECORD_CAP,
                deadline,
                scan,
            )?;
            if jstr(&network, "Id", 64)? != r.internal_network_id
                || !jbool(&network, "Internal")?
                || jstr(&network, "Scope", 32)? != "local"
                || jstr(&network, "Driver", 64)? != "bridge"
                || network["Labels"]["com.docker.compose.project"].as_str()
                    != Some(&r.compose_project)
            {
                return Err(invalid());
            }
            let joined = network["Containers"].as_object().ok_or_else(invalid)?;
            if joined.len() != 1 || !joined.contains_key(&r.pg_container_id) {
                return Err(invalid());
            }
            let ports = own["NetworkSettings"]["Ports"]
                .as_object()
                .ok_or_else(invalid)?;
            if ports
                .values()
                .any(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()))
            {
                return Err(invalid());
            }
            let host = &own["HostConfig"];
            if jbool(host, "Privileged")?
                || jbool(host, "PublishAllPorts")?
                || !jstr(host, "PidMode", 32)?.is_empty()
                || !jstr(host, "UsernsMode", 32)?.is_empty()
                || ![network_name.as_str(), r.internal_network_id.as_str()].contains(&jstr(
                    host,
                    "NetworkMode",
                    256,
                )?)
            {
                return Err(invalid());
            }
            if !host["PortBindings"].is_null()
                && host["PortBindings"]
                    .as_object()
                    .is_none_or(|m| !m.is_empty())
            {
                return Err(invalid());
            }
            let image = self.query(
                &format!("/v1.45/images/{}/json", r.admitted_pg_image_id),
                RECORD_CAP,
                deadline,
                scan,
            )?;
            if jstr(&image, "Id", 71)? != r.admitted_pg_image_id
                || !image["RepoDigests"].as_array().is_some_and(|a| {
                    a.iter().any(|s| {
                        s.as_str().is_some_and(|s| {
                            s.strip_suffix(PG_DIGEST).is_some_and(|p| p.ends_with('@'))
                        })
                    })
                })
            {
                return Err(invalid());
            }
            let mountinfo = proc.read_relative(
                &format!("{}/mountinfo", current.process_id),
                RECORD_CAP,
                scan,
            )?;
            require_twelve_mountinfo(&mountinfo, scan)?;
            let assets_index = current
                .mounts
                .iter()
                .position(|m| m.destination == "/var/lib/knowweave-source/assets")
                .ok_or_else(invalid)?;
            let raw_assets = own["Mounts"]
                .as_array()
                .ok_or_else(invalid)?
                .iter()
                .find(|m| {
                    m["Destination"].as_str()
                        == Some(current.mounts[assets_index].destination.as_str())
                })
                .ok_or_else(invalid)?;
            let assets: ActualAssetsMount = serde_json::from_value(raw_assets.clone())?;
            assets.validate()?;
            let export = mapped(&current.mounts, InstalledSnapshotBinding::EXPORT_PATH)?;
            let companion = mapped(&current.mounts, InstalledSnapshotBinding::COMPANION_PATH)?;
            for row in &rows {
                if row.container_id != r.pg_container_id {
                    if row.project.as_deref() == Some(&r.compose_project) && row.running {
                        return Err(invalid());
                    }
                    if row.mounts.iter().any(|m| {
                        m.rw && [&export, &companion, &assets.source]
                            .into_iter()
                            .any(|p| contains(p, &m.source) || contains(&m.source, p))
                    }) {
                        return Err(invalid());
                    }
                }
            }
            let legacy = rows
                .iter()
                .filter(|c| {
                    c.container_id != r.pg_container_id
                        && c.project.as_deref() == Some(&r.compose_project)
                })
                .map(|c| clone_data(c, RECORD_CAP, scan))
                .collect::<Result<Vec<_>>>()?;
            let current_mounts = current
                .mounts
                .iter()
                .map(|m| clone_data(m, 16384, scan))
                .collect::<Result<Vec<_>>>()?;
            let pg_pid = u32::try_from(current.process_id).map_err(|_| invalid())?;
            pay(scan, 16384, 32)?;
            let security = encode(
                &serde_json::json!({"host":host,"network_attachment":attachment,"config_user":own["Config"]["User"],"mounts":own["Mounts"]}),
                RECORD_CAP,
                scan,
            )?;
            let data = HostCutoverData {
                container_id: r.pg_container_id.clone(),
                image_sha256: r.admitted_pg_image_id.clone(),
                export_mount: export,
                journal_mount: companion,
                assets_mount: assets,
                legacy_producers: legacy,
                complete_container_projection: rows,
                daemon_id: r.daemon_id.clone(),
                mountinfo_sha256: hash(&mountinfo, scan)?,
                docker_cli_sha256: self.cli_digest.clone(),
            };
            data.validate()?;
            Ok(HostObservation {
                data,
                network: encode(&network, RECORD_CAP, scan)?,
                image: encode(&image, RECORD_CAP, scan)?,
                security,
                pg_pid,
                current_mounts,
            })
        }
    }
    fn assemble_original_host(
        pack: &FixedHostPack,
        catalog: &job::ExecutionCatalog,
        deadline: Instant,
        proc: &ProcView,
        cli_digest: &str,
        scan: &SharedJournalBudget,
    ) -> Result<HostObservation> {
        let r = &catalog.installed_runtime;
        let info = pack.info.as_ref().ok_or_else(invalid)?;
        if jstr(info, "ID", 128)? != r.daemon_id {
            return Err(invalid());
        }
        let listed = pack.listed.as_ref().ok_or_else(invalid)?;
        let all = listed.as_array().ok_or_else(invalid)?;
        if all.is_empty() || all.len() > CONTAINER_CAP {
            return Err(invalid());
        }
        pay(scan, all.len() * 128, all.len())?;
        let mut ids = all
            .iter()
            .map(|x| jstr(x, "Id", 64).map(str::to_owned))
            .collect::<Result<Vec<_>>>()?;
        ids.sort();
        if ids.iter().any(|x| !crate::valid_digest(x)) || ids.windows(2).any(|x| x[0] == x[1]) {
            return Err(invalid());
        }
        let mut rows = Vec::with_capacity(ids.len());
        let mut own = None;
        for id in ids {
            root_io_boundary(deadline, &STOP)?;
            let raw = pack.containers.get(&id).ok_or_else(invalid)?;
            let row = projection(raw, scan)?;
            if row.container_id != id {
                return Err(invalid());
            }
            if id == r.pg_container_id {
                own = Some(raw);
            }
            rows.push(row);
        }
        let own = own.ok_or_else(invalid)?;
        let current = rows
            .iter()
            .find(|c| c.container_id == r.pg_container_id)
            .ok_or_else(invalid)?;
        if !current.running
            || current.image_sha256 != r.admitted_pg_image_id
            || current.project.as_deref() != Some(&r.compose_project)
            || current.service.as_deref() != Some("pg")
            || current.mounts.len() != 12
            || jstr(&own["Config"], "Image", 256)? != PG_IMAGE
        {
            return Err(invalid());
        }
        for (m, (destination, kind, rw)) in current.mounts.iter().zip(TOPOLOGY) {
            if m.destination != destination
                || m.kind != kind
                || m.rw != rw
                || (kind == "volume" && m.name.as_ref().is_none_or(String::is_empty))
            {
                return Err(invalid());
            }
        }
        let networks = own["NetworkSettings"]["Networks"]
            .as_object()
            .ok_or_else(invalid)?;
        if networks.len() != 1 {
            return Err(invalid());
        }
        let (network_name, attachment) = networks.iter().next().ok_or_else(invalid)?;
        if jstr(attachment, "NetworkID", 64)? != r.internal_network_id {
            return Err(invalid());
        }
        let network = pack.network.as_ref().ok_or_else(invalid)?;
        if jstr(network, "Id", 64)? != r.internal_network_id
            || !jbool(network, "Internal")?
            || jstr(network, "Scope", 32)? != "local"
            || jstr(network, "Driver", 64)? != "bridge"
            || network["Labels"]["com.docker.compose.project"].as_str() != Some(&r.compose_project)
        {
            return Err(invalid());
        }
        let joined = network["Containers"].as_object().ok_or_else(invalid)?;
        if joined.len() != 1 || !joined.contains_key(&r.pg_container_id) {
            return Err(invalid());
        }
        let ports = own["NetworkSettings"]["Ports"]
            .as_object()
            .ok_or_else(invalid)?;
        if ports
            .values()
            .any(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()))
        {
            return Err(invalid());
        }
        let host = &own["HostConfig"];
        if jbool(host, "Privileged")?
            || jbool(host, "PublishAllPorts")?
            || !jstr(host, "PidMode", 32)?.is_empty()
            || !jstr(host, "UsernsMode", 32)?.is_empty()
            || ![network_name.as_str(), r.internal_network_id.as_str()].contains(&jstr(
                host,
                "NetworkMode",
                256,
            )?)
        {
            return Err(invalid());
        }
        if !host["PortBindings"].is_null()
            && host["PortBindings"]
                .as_object()
                .is_none_or(|m| !m.is_empty())
        {
            return Err(invalid());
        }
        let image = pack.image.as_ref().ok_or_else(invalid)?;
        if jstr(image, "Id", 71)? != r.admitted_pg_image_id
            || !image["RepoDigests"].as_array().is_some_and(|a| {
                a.iter().any(|s| {
                    s.as_str().is_some_and(|s| {
                        s.strip_suffix(PG_DIGEST).is_some_and(|p| p.ends_with('@'))
                    })
                })
            })
        {
            return Err(invalid());
        }
        let mountinfo = proc.read_relative(
            &format!("{}/mountinfo", current.process_id),
            RECORD_CAP,
            scan,
        )?;
        require_twelve_mountinfo(&mountinfo, scan)?;
        let assets_index = current
            .mounts
            .iter()
            .position(|m| m.destination == "/var/lib/knowweave-source/assets")
            .ok_or_else(invalid)?;
        let raw_assets = own["Mounts"]
            .as_array()
            .ok_or_else(invalid)?
            .iter()
            .find(|m| {
                m["Destination"].as_str() == Some(current.mounts[assets_index].destination.as_str())
            })
            .ok_or_else(invalid)?;
        let assets: ActualAssetsMount = serde_json::from_value(raw_assets.clone())?;
        assets.validate()?;
        let export = mapped(&current.mounts, InstalledSnapshotBinding::EXPORT_PATH)?;
        let companion = mapped(&current.mounts, InstalledSnapshotBinding::COMPANION_PATH)?;
        for row in &rows {
            if row.container_id != r.pg_container_id {
                if row.project.as_deref() == Some(&r.compose_project) && row.running {
                    return Err(invalid());
                }
                if row.mounts.iter().any(|m| {
                    m.rw && [&export, &companion, &assets.source]
                        .into_iter()
                        .any(|p| contains(p, &m.source) || contains(&m.source, p))
                }) {
                    return Err(invalid());
                }
            }
        }
        let legacy = rows
            .iter()
            .filter(|c| {
                c.container_id != r.pg_container_id
                    && c.project.as_deref() == Some(&r.compose_project)
            })
            .map(|c| clone_data(c, RECORD_CAP, scan))
            .collect::<Result<Vec<_>>>()?;
        let current_mounts = current
            .mounts
            .iter()
            .map(|m| clone_data(m, 16384, scan))
            .collect::<Result<Vec<_>>>()?;
        let pg_pid = u32::try_from(current.process_id).map_err(|_| invalid())?;
        pay(scan, 16384, 32)?;
        let security = encode(
            &serde_json::json!({"host":host,"network_attachment":attachment,"config_user":own["Config"]["User"],"mounts":own["Mounts"]}),
            RECORD_CAP,
            scan,
        )?;
        let data = HostCutoverData {
            container_id: r.pg_container_id.clone(),
            image_sha256: r.admitted_pg_image_id.clone(),
            export_mount: export,
            journal_mount: companion,
            assets_mount: assets,
            legacy_producers: legacy,
            complete_container_projection: rows,
            daemon_id: r.daemon_id.clone(),
            mountinfo_sha256: hash(&mountinfo, scan)?,
            docker_cli_sha256: cli_digest.to_owned(),
        };
        data.validate()?;
        Ok(HostObservation {
            data,
            network: encode(&network, RECORD_CAP, scan)?,
            image: encode(&image, RECORD_CAP, scan)?,
            security,
            pg_pid,
            current_mounts,
        })
    }

    fn require_twelve_mountinfo(raw: &[u8], scan: &SharedJournalBudget) -> Result<()> {
        pay(scan, raw.len() * 8 + 1024, 0)?;
        let text = std::str::from_utf8(raw).map_err(|_| invalid())?;
        let mut count = [0_usize; 12];
        for line in text.lines() {
            pay(scan, 0, 8)?;
            let (left, _) = line.split_once(" - ").ok_or_else(invalid)?;
            let mut fields = left.split_ascii_whitespace();
            let mut f = [""; 6];
            for slot in &mut f {
                *slot = fields.next().ok_or_else(invalid)?;
            }
            let target = f[4];
            if let Some((i, (_, _, rw))) = TOPOLOGY.iter().enumerate().find(|(_, x)| x.0 == target)
            {
                count[i] += 1;
                if f[5].split(',').any(|x| x == "rw") != *rw
                    || f[5].split(',').any(|x| x == "ro") == *rw
                {
                    return Err(invalid());
                }
            } else if TOPOLOGY
                .iter()
                .any(|(p, _, _)| contains(p, target) && *p != target)
            {
                return Err(invalid());
            }
        }
        if count != [1; 12] {
            return Err(invalid());
        }
        Ok(())
    }
    struct PgCustody {
        pid: u32,
        start: u64,
        root: Arc<File>,
        root_object: NativeObject,
        user: TaskNamespace,
        pidns: Arc<File>,
        mntns: Arc<File>,
        pid_name: String,
        mnt_name: String,
        exe: File,
        exe_stamp: Stamp,
        exe_digest: [u8; 32],
        data: File,
        socket: File,
        directory: File,
        data_object: NativeObject,
        socket_object: NativeObject,
        directory_object: NativeObject,
        pidfd: ParentPid,
    }
    impl PgCustody {
        fn open(
            proc: &ProcView,
            pid: u32,
            catalog: &job::ExecutionCatalog,
            scan: &SharedJournalBudget,
        ) -> Result<Self> {
            pay(scan, 4096, 32)?;
            if process_status(proc, pid, 999, scan)? != 1 {
                return Err(invalid());
            }
            let stat = proc.read_relative(&format!("{pid}/stat"), 4096, scan)?;
            let start = parse_start(&stat, pid)?;
            let user = TaskNamespace::observe(proc, pid, scan)?;
            let root = share_native(proc.open_checked_magic(&format!("{pid}/root"), scan)?, scan)?;
            let root_object = actual_object(&root, scan)?;
            let pidns = share_native(
                proc.open_checked_magic(&format!("{pid}/ns/pid"), scan)?,
                scan,
            )?;
            let mntns = share_native(
                proc.open_checked_magic(&format!("{pid}/ns/mnt"), scan)?,
                scan,
            )?;
            let pid_name = proc.read_link_relative(&format!("{pid}/ns/pid"), scan)?;
            let mnt_name = proc.read_link_relative(&format!("{pid}/ns/mnt"), scan)?;
            if pid_name != catalog.installed_runtime.pg_pid_namespace
                || mnt_name != catalog.installed_runtime.pg_mount_namespace
                || start != catalog.installed_runtime.postmaster_start_ticks
            {
                return Err(invalid());
            }
            let mut exe = proc.open_checked_magic(&format!("{pid}/exe"), scan)?;
            let exe_stamp = Stamp::observe(&exe, scan)?;
            let actual = open_beneath(
                &root,
                "/usr/lib/postgresql/18/bin/postgres",
                libc::O_RDONLY,
                scan,
            )?;
            if exe_stamp != Stamp::observe(&actual, scan)?
                || !exe_stamp.root_file(128 * 1024 * 1024)
            {
                return Err(invalid());
            }
            let raw = read_exact_file(&mut exe, exe_stamp.bytes, scan)?;
            if !raw.starts_with(b"\x7fELF") {
                return Err(invalid());
            }
            let exe_digest = digest_bytes(&raw, scan)?;
            let data = open_beneath(
                &root,
                "/var/lib/postgresql",
                libc::O_PATH | libc::O_DIRECTORY,
                scan,
            )?;
            let directory = open_beneath(
                &root,
                "/run/postgresql",
                libc::O_PATH | libc::O_DIRECTORY,
                scan,
            )?;
            let socket = open_beneath(&root, "/run/postgresql/.s.PGSQL.5432", libc::O_PATH, scan)?;
            let data_object = actual_object(&data, scan)?;
            let socket_object = actual_object(&socket, scan)?;
            let directory_object = actual_object(&directory, scan)?;
            let endpoint = &catalog.peer_endpoint;
            if data_object.kind != libc::S_IFDIR
                || directory_object.kind != libc::S_IFDIR
                || socket_object.kind != libc::S_IFSOCK
                || (directory_object.dev, directory_object.ino)
                    != (endpoint.directory_dev, endpoint.directory_ino)
                || (socket_object.dev, socket_object.ino)
                    != (endpoint.socket_dev, endpoint.socket_ino)
                || Stamp::observe(&socket, scan)?.uid != 999
            {
                return Err(invalid());
            }
            let pidfd = ParentPid::open(pid, start, scan)?;
            let value = Self {
                pid,
                start,
                root,
                root_object,
                user,
                pidns,
                mntns,
                pid_name,
                mnt_name,
                exe,
                exe_stamp,
                exe_digest,
                data,
                socket,
                directory,
                data_object,
                socket_object,
                directory_object,
                pidfd,
            };
            value.recheck(proc, scan)?;
            Ok(value)
        }
        fn recheck(&self, proc: &ProcView, scan: &SharedJournalBudget) -> Result<()> {
            self.pidfd.revalidate(scan)?;
            self.user.revalidate(proc, scan)?;
            if process_status(proc, self.pid, 999, scan)? != 1 {
                return Err(invalid());
            }
            let root = proc.open_checked_magic(&format!("{}/root", self.pid), scan)?;
            if actual_object(&root, scan)? != self.root_object
                || actual_object(&self.root, scan)? != self.root_object
            {
                return Err(invalid());
            }
            drop(root);
            for (leaf, file, name) in [
                ("pid", &self.pidns, &self.pid_name),
                ("mnt", &self.mntns, &self.mnt_name),
            ] {
                let named = proc.open_checked_magic(&format!("{}/ns/{leaf}", self.pid), scan)?;
                if actual_object(file, scan)? != actual_object(&named, scan)?
                    || proc.read_link_relative(&format!("{}/ns/{leaf}", self.pid), scan)? != *name
                {
                    return Err(invalid());
                }
            }
            if Stamp::observe(&self.exe, scan)? != self.exe_stamp
                || Stamp::observe(
                    &proc.open_checked_magic(&format!("{}/exe", self.pid), scan)?,
                    scan,
                )? != self.exe_stamp
            {
                return Err(invalid());
            }
            for (path, held, object) in [
                ("/var/lib/postgresql", &self.data, self.data_object),
                ("/run/postgresql", &self.directory, self.directory_object),
                (
                    "/run/postgresql/.s.PGSQL.5432",
                    &self.socket,
                    self.socket_object,
                ),
            ] {
                let named = open_beneath(&self.root, path, libc::O_PATH, scan)?;
                if actual_object(held, scan)? != object || actual_object(&named, scan)? != object {
                    return Err(invalid());
                }
            }
            Ok(())
        }
        fn native(&self) -> job::Native {
            job::Native {
                postmaster_start_ticks: self.start,
                data_dev: self.data_object.dev,
                data_ino: self.data_object.ino,
                socket_dev: self.socket_object.dev,
                socket_ino: self.socket_object.ino,
            }
        }
        fn check_postmaster_start(&self, value: &str, scan: &SharedJournalBudget) -> Result<()> {
            if value.len() > 128 {
                return Err(invalid());
            }
            pay(scan, value.len() * 16 + 8192, 16)?;
            let timestamp =
                sqlx::types::chrono::DateTime::parse_from_rfc3339(value).map_err(|_| invalid())?;
            let path = "/var/lib/postgresql/18/docker/postmaster.pid";
            let mut file = open_beneath(&self.root, path, libc::O_RDONLY, scan)?;
            let stamp = Stamp::observe(&file, scan)?;
            if stamp.uid != 999
                || stamp.gid != 999
                || stamp.mode & libc::S_IFMT != libc::S_IFREG
                || stamp.mode & 0o7777 != 0o600
                || stamp.links != 1
                || stamp.bytes == 0
                || stamp.bytes > 4096
            {
                return Err(invalid());
            }
            let raw = read_exact_file(&mut file, stamp.bytes, scan)?;
            pay(scan, raw.len() * 8, 16)?;
            let text = std::str::from_utf8(&raw).map_err(|_| invalid())?;
            let mut rows = text.lines();
            let mut lines = [""; 4];
            for slot in &mut lines {
                *slot = rows.next().ok_or_else(invalid)?;
            }
            if lines[0] != "1" || lines[1] != "/var/lib/postgresql/18/docker" || lines[3] != "5432"
            {
                return Err(invalid());
            }
            let seconds = lines[2].parse::<i64>().map_err(|_| invalid())?;
            if seconds <= 0 || seconds.to_string() != lines[2] || timestamp.timestamp() != seconds {
                return Err(invalid());
            }
            if Stamp::observe(&file, scan)? != stamp {
                return Err(invalid());
            }
            drop(file);
            if Stamp::observe(&open_beneath(&self.root, path, libc::O_RDONLY, scan)?, scan)?
                != stamp
            {
                return Err(invalid());
            }
            Ok(())
        }
        fn native_root_candidates(
            &self,
            proc: &ProcView,
            terminal_capture: Option<(u32, u64)>,
            scan: &SharedJournalBudget,
        ) -> Result<Vec<(u32, u64, Stamp)>> {
            // Full native host process set; no name-only or UID-only admission.
            pay(scan, 65536, 4)?;
            // The literal proc root is already held; getdents uses an independent
            // open file description and cannot change any retained task offset.
            let raw = unsafe {
                libc::openat(
                    proc.root.as_raw_fd(),
                    c".".as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                )
            };
            if raw < 0 {
                return Err(invalid());
            }
            let directory = unsafe { File::from_raw_fd(raw) };
            let mut b = [0_u8; 65536];
            let mut pids = Vec::new();
            loop {
                pay(scan, b.len(), 1)?;
                let n = unsafe {
                    libc::syscall(
                        libc::SYS_getdents64,
                        directory.as_raw_fd(),
                        b.as_mut_ptr(),
                        b.len(),
                    )
                };
                if n < 0 {
                    return Err(invalid());
                }
                if n == 0 {
                    break;
                }
                let mut i = 0;
                while i < n as usize {
                    if i + 19 > n as usize {
                        return Err(invalid());
                    }
                    let size = u16::from_ne_bytes([b[i + 16], b[i + 17]]) as usize;
                    if size < 20 || i + size > n as usize {
                        return Err(invalid());
                    }
                    let name = &b[i + 19..i + size];
                    let end = name.iter().position(|x| *x == 0).ok_or_else(invalid)?;
                    let name = &name[..end];
                    pay(scan, name.len() * 2 + 32, 1)?;
                    if !name.is_empty() && name.iter().all(u8::is_ascii_digit) {
                        if pids.len() >= ENTRY_CAP {
                            return Err(invalid());
                        }
                        pids.push(
                            std::str::from_utf8(name)
                                .map_err(|_| invalid())?
                                .parse::<u32>()
                                .map_err(|_| invalid())?,
                        );
                    }
                    i += size;
                }
            }
            drop(directory);
            pids.sort_unstable();
            if pids.windows(2).any(|w| w[0] == w[1]) {
                return Err(invalid());
            }

            let mut roots = Vec::new();
            for pid in pids {
                // Only the already-signalled original Capture pidfd permits
                // this original15/16 settlement observation. A same numeric
                // PID with another start, or a still-live task, is rejected.
                // Dead task namespace/exe links may no longer be readable.
                if let Some((original, start)) = terminal_capture
                    && pid == original
                {
                    let stat = proc.read_relative(&format!("{pid}/stat"), 4096, scan)?;
                    terminal_capture_stat_data(&stat, pid, start)?;
                    continue;
                }
                let ns = proc.read_link_relative(&format!("{pid}/ns/pid"), scan)?;
                if ns != self.pid_name {
                    continue;
                }
                let start = actual_start(pid, scan)?;
                let (uid, _) = process_identity_status(proc, pid, scan)?;
                if ![0, 999].contains(&uid) {
                    return Err(invalid());
                }
                let exe = proc.open_checked_magic(&format!("{pid}/exe"), scan)?;
                let stamp = Stamp::observe(&exe, scan)?;
                if actual_start(pid, scan)? != start {
                    return Err(invalid());
                }
                if uid == 0 {
                    pay(scan, 256, 1)?;
                    roots.push((pid, start, stamp));
                } else if stamp != self.exe_stamp {
                    return Err(invalid());
                }
            }
            Ok(roots)
        }
        fn no_other_admin(
            &self,
            proc: &ProcView,
            capture: u32,
            scan: &SharedJournalBudget,
        ) -> Result<()> {
            for (pid, _, _) in self.native_root_candidates(proc, None, scan)? {
                if pid != capture {
                    return Err(invalid());
                }
            }
            Ok(())
        }
    }
    // These are the three existing private M3 tool recipes. A match is only
    // process identity DATA; it never attests the SQL result or random keys.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum OriginalPgHelperKind {
        Observer,
        DumpVersion,
        Dump,
    }
    impl OriginalPgHelperKind {
        fn executable(self) -> &'static str {
            match self {
                Self::Observer => "/usr/lib/postgresql/18/bin/psql",
                Self::DumpVersion | Self::Dump => "/usr/lib/postgresql/18/bin/pg_dump",
            }
        }
    }
    fn canonical_u32(text: &str) -> Result<u32> {
        if text.is_empty() || text.len() > 10 || !text.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid());
        }
        let value = text.parse::<u32>().map_err(|_| invalid())?;
        if value.to_string() != text {
            return Err(invalid());
        }
        Ok(value)
    }
    fn original_observer_sql_shape(sql: &str) -> Result<()> {
        const PREFIX: &str = "SELECT pg_catalog.pg_backend_pid()::text||'|'||backend_start::text FROM pg_catalog.pg_stat_activity WHERE pid=pg_catalog.pg_backend_pid();BEGIN READ ONLY; SET LOCAL statement_timeout = '5000ms'; SELECT current_user::text || '|' || pg_catalog.current_database() || '|' || d.oid::text || '|' || pcs.system_identifier::text FROM pg_catalog.pg_database d CROSS JOIN pg_catalog.pg_control_system() pcs WHERE d.datname=pg_catalog.current_database(); SELECT pid, database::bigint, classid::bigint, objid::bigint, objsubid, mode, granted FROM pg_catalog.pg_locks WHERE locktype='advisory' AND (classid::bigint,objid::bigint) IN ((";
        let values = sql
            .strip_prefix(PREFIX)
            .and_then(|v| v.strip_suffix(")); ROLLBACK;"))
            .ok_or_else(invalid)?;
        let (first, second) = values.split_once("),(").ok_or_else(invalid)?;
        let (a, b) = first.split_once(',').ok_or_else(invalid)?;
        let (c, d) = second.split_once(',').ok_or_else(invalid)?;
        for part in [a, b, c, d] {
            let _ = canonical_u32(part)?;
        }
        Ok(())
    }
    fn original_pg_helper_recipe(
        raw: &[u8],
        database: &str,
        scan: &SharedJournalBudget,
    ) -> Result<OriginalPgHelperKind> {
        if raw.is_empty() || raw.len() > 8192 || raw.last() != Some(&0) || !job::database(database)
        {
            return Err(invalid());
        }
        pay(scan, raw.len() * 16 + 4096, 32)?;
        // Twelve is the longest already-fixed recipe, not an input-selected
        // allocation cap. Refuse a thirteenth argument before collecting it.
        let mut slots = [""; 12];
        let mut count = 0_usize;
        for part in raw[..raw.len() - 1].split(|b| *b == 0) {
            if count == slots.len() {
                return Err(invalid());
            }
            slots[count] = std::str::from_utf8(part).map_err(|_| invalid())?;
            count += 1;
        }
        let args = &slots[..count];
        if args == ["/usr/lib/postgresql/18/bin/pg_dump", "--version"] {
            return Ok(OriginalPgHelperKind::DumpVersion);
        }
        let db = format!("--dbname={database}");
        if args
            == [
                "/usr/lib/postgresql/18/bin/pg_dump",
                "--format=custom",
                "--no-password",
                "--lock-wait-timeout=5000",
                "--host=/var/run/postgresql",
                "--port=5432",
                "--username=learning_admin",
                db.as_str(),
            ]
        {
            return Ok(OriginalPgHelperKind::Dump);
        }
        if args.len() == 12
            && args[..11]
                == [
                    "/usr/lib/postgresql/18/bin/psql",
                    "-XAt",
                    "--no-password",
                    "--host=/var/run/postgresql",
                    "--port=5432",
                    "--username=learning_admin",
                    db.as_str(),
                    "-q",
                    "-v",
                    "ON_ERROR_STOP=1",
                    "-c",
                ]
        {
            original_observer_sql_shape(args[11])?;
            return Ok(OriginalPgHelperKind::Observer);
        }
        Err(invalid())
    }
    fn actual_parent(
        proc: &ProcView,
        pid: u32,
        start: u64,
        scan: &SharedJournalBudget,
    ) -> Result<u32> {
        let raw = proc.read_relative(&format!("{pid}/stat"), 4096, scan)?;
        if parse_start(&raw, pid)? != start {
            return Err(invalid());
        }
        pay(scan, raw.len() * 4 + 128, 4)?;
        let text = std::str::from_utf8(&raw).map_err(|_| invalid())?;
        let (_, tail) = text.rsplit_once(')').ok_or_else(invalid)?;
        let parent = tail.split_ascii_whitespace().nth(1).ok_or_else(invalid)?;
        let parent = canonical_u32(parent)?;
        if parent == 0 || parent == pid {
            return Err(invalid());
        }
        Ok(parent)
    }
    fn terminal_capture_stat_data(raw: &[u8], pid: u32, start: u64) -> Result<()> {
        if parse_start(raw, pid)? != start {
            return Err(invalid());
        }
        let text = std::str::from_utf8(raw).map_err(|_| invalid())?;
        let (_, tail) = text.rsplit_once(')').ok_or_else(invalid)?;
        if !matches!(tail.split_ascii_whitespace().next(), Some("Z" | "X")) {
            return Err(invalid());
        }
        Ok(())
    }
    fn original_tool_environment(
        proc: &ProcView,
        pid: u32,
        scan: &SharedJournalBudget,
    ) -> Result<()> {
        original_environment(
            proc,
            pid,
            &[
                ("LC_ALL", "C"),
                ("PGCONNECT_TIMEOUT", "5"),
                ("PGPASSFILE", "/dev/null"),
                ("PGSSLMODE", "disable"),
            ],
            scan,
        )
    }
    fn original_environment<const N: usize>(
        proc: &ProcView,
        pid: u32,
        allowed: &[(&str, &str); N],
        scan: &SharedJournalBudget,
    ) -> Result<()> {
        // Read keys one byte at a time. An unknown key is rejected at '=';
        // its value is never read, retained, logged or compared.
        fd_gate(2, scan)?;
        let file = proc.open_relative(
            &format!("{pid}/environ"),
            libc::O_RDONLY | libc::O_NOFOLLOW,
            scan,
        )?;
        let mut seen = [false; N];
        let next = |scan: &SharedJournalBudget| -> Result<Option<u8>> {
            loop {
                pay(scan, 1, 1)?;
                let mut byte = 0_u8;
                let n = unsafe { libc::read(file.as_raw_fd(), (&mut byte as *mut u8).cast(), 1) };
                if n == 1 {
                    return Ok(Some(byte));
                }
                if n == 0 {
                    return Ok(None);
                }
                if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                    return Err(invalid());
                }
            }
        };
        for item in 0..=N {
            let Some(first) = next(scan)? else {
                return if item == N && seen.into_iter().all(|v| v) {
                    Ok(())
                } else {
                    Err(invalid())
                };
            };
            if item == N {
                return Err(invalid());
            }
            let mut key = [0_u8; 32];
            key[0] = first;
            let mut len = 1_usize;
            loop {
                let byte = next(scan)?.ok_or_else(invalid)?;
                if byte == b'=' {
                    break;
                }
                if byte == 0 || len == key.len() {
                    return Err(invalid());
                }
                key[len] = byte;
                len += 1;
            }
            let index = allowed
                .iter()
                .position(|(name, _)| name.as_bytes() == &key[..len])
                .ok_or_else(invalid)?;
            if seen[index] {
                return Err(invalid());
            }
            seen[index] = true;
            for expected in allowed[index]
                .1
                .as_bytes()
                .iter()
                .copied()
                .chain(std::iter::once(0))
            {
                if next(scan)? != Some(expected) {
                    return Err(invalid());
                }
            }
        }
        Err(invalid())
    }
    fn original_helper_paths(
        raw: &[u8],
        executable: &str,
        scan: &SharedJournalBudget,
    ) -> Result<Vec<String>> {
        pay(scan, raw.len() * 8 + 4096, 4)?;
        let text = std::str::from_utf8(raw).map_err(|_| invalid())?;
        let mut paths = BTreeSet::new();
        for line in text.lines() {
            pay(scan, 0, 8)?;
            let mut columns = line.split_ascii_whitespace();
            let mut fields = [""; 6];
            for slot in &mut fields[..5] {
                *slot = columns.next().ok_or_else(invalid)?;
            }
            if fields[1].len() != 4 {
                return Err(invalid());
            }
            if fields[1].as_bytes()[2] != b'x' {
                continue;
            }
            fields[5] = columns.next().ok_or_else(invalid)?;
            if columns.next().is_some()
                || fields[1].as_bytes()[1] == b'w'
                || fields[1].as_bytes()[3] != b'p'
            {
                return Err(invalid());
            }
            let path = fields[5];
            if ["[vdso]", "[vsyscall]"].contains(&path) {
                continue;
            }
            if !job::path(path)
                || path != executable
                    && (!(path.starts_with("/usr/lib/") || path.starts_with("/lib/"))
                        || !path.contains(".so"))
            {
                return Err(invalid());
            }
            if paths.len() >= 32 && !paths.contains(path) {
                return Err(invalid());
            }
            pay(scan, path.len() * 2 + 128, 2)?;
            paths.insert(path.to_owned());
        }
        if !paths.contains(executable) {
            return Err(invalid());
        }
        pay(
            scan,
            paths.len() * std::mem::size_of::<String>(),
            paths.len(),
        )?;
        Ok(paths.into_iter().collect())
    }
    impl MeasuredCode {
        fn open_native_helper(
            root: Arc<File>,
            path: &str,
            scan: &SharedJournalBudget,
        ) -> Result<Self> {
            // No invented RoleImage or Catalog digest. This actual named ELF
            // is measured under the original admitted PG root/image closure.
            // C must separately prove that closure before any real enablement.
            let binding = CodeBinding::open(root, path, scan)?;
            let mut file = binding.open_leaf(scan)?;
            let stamp = Stamp::observe(&file, scan)?;
            if !stamp.root_file(128 * 1024 * 1024) || stamp.gid != 0 {
                return Err(invalid());
            }
            let raw = read_exact_file(&mut file, stamp.bytes, scan)?;
            if !raw.starts_with(b"\x7fELF") {
                return Err(invalid());
            }
            let digest = digest_bytes(&raw, scan)?;
            pay(scan, path.len() + std::mem::size_of::<Self>(), 1)?;
            let measured = Self {
                file,
                binding,
                path: path.to_owned(),
                stamp,
                digest,
            };
            measured.recheck(scan)?;
            Ok(measured)
        }
    }
    struct OriginalPgHelper {
        pid: u32,
        start: u64,
        parent: u32,
        parent_start: u64,
        pidfd: ParentPid,
        kind: OriginalPgHelperKind,
        command: Vec<u8>,
        codes: Vec<Arc<MeasuredCode>>,
        mappings: Vec<String>,
    }
    fn helper_namespace(
        proc: &ProcView,
        pid: u32,
        pg: &PgCustody,
        capture: &MeasuredProcess,
        scan: &SharedJournalBudget,
    ) -> Result<()> {
        if process_status(proc, pid, 0, scan)? <= 1 {
            return Err(invalid());
        }
        for (leaf, expected) in [("pid", &pg.pidns), ("mnt", &pg.mntns)] {
            let named = proc.open_checked_magic(&format!("{pid}/ns/{leaf}"), scan)?;
            if actual_object(&named, scan)? != actual_object(expected, scan)? {
                return Err(invalid());
            }
        }
        let user = TaskNamespace::observe(proc, pid, scan)?;
        if !user.object.same_object(&capture.user.object)
            || !user.object.same_object(&pg.user.object)
        {
            return Err(invalid());
        }
        drop(user);
        let root = proc.open_checked_magic(&format!("{pid}/root"), scan)?;
        if actual_object(&root, scan)? != pg.root_object || capture.root_object != pg.root_object {
            return Err(invalid());
        }
        Ok(())
    }
    impl OriginalPgHelper {
        fn open(
            proc: &ProcView,
            task: (u32, u64),
            pg: &PgCustody,
            capture: &MeasuredProcess,
            database: &str,
            cache: &mut Vec<Arc<MeasuredCode>>,
            scan: &SharedJournalBudget,
        ) -> Result<Self> {
            let (pid, start) = task;
            fd_gate(4, scan)?;
            capture.recheck(proc, scan)?;
            pg.recheck(proc, scan)?;
            if actual_parent(proc, pid, start, scan)? != capture.pid {
                return Err(invalid());
            }
            helper_namespace(proc, pid, pg, capture, scan)?;
            fd_gate(2, scan)?;
            let pidfd = ParentPid::open(pid, start, scan)?;
            let command = proc.read_relative(&format!("{pid}/cmdline"), 8192, scan)?;
            let kind = original_pg_helper_recipe(&command, database, scan)?;
            original_tool_environment(proc, pid, scan)?;
            let raw = proc.read_relative(&format!("{pid}/maps"), 1024 * 1024, scan)?;
            let paths = original_helper_paths(&raw, kind.executable(), scan)?;
            pay(
                scan,
                capture.codes.len() * std::mem::size_of::<Arc<MeasuredCode>>() + 4096,
                capture.codes.len() + paths.len(),
            )?;
            // The two fixed main tools have no invented Catalog RoleImage.
            // Their actual main ELF is joined to the admitted PG root/image;
            // each dependency must already be in Capture's approved Catalog
            // CodeFile closure, not merely a new observed/hashable .so file.
            if paths.iter().any(|path| {
                path != kind.executable()
                    && !capture.codes[1..].iter().any(|code| code.path == *path)
            }) {
                return Err(invalid());
            }
            let main =
                if let Some(original) = cache.iter().find(|code| code.path == kind.executable()) {
                    original.recheck(scan)?;
                    Arc::clone(original)
                } else {
                    if cache.len() >= 2 {
                        return Err(invalid());
                    }
                    fd_gate(2, scan)?;
                    let code = Arc::new(MeasuredCode::open_native_helper(
                        Arc::clone(&pg.root),
                        kind.executable(),
                        scan,
                    )?);
                    cache.push(Arc::clone(&code));
                    code
                };
            let mut codes = Vec::with_capacity(capture.codes.len());
            codes.push(main);
            codes.extend(capture.codes[1..].iter().map(Arc::clone));
            let named = proc.open_checked_magic(&format!("{pid}/exe"), scan)?;
            let main = codes
                .iter()
                .find(|c| c.path == kind.executable())
                .ok_or_else(invalid)?;
            if Stamp::observe(&named, scan)? != main.stamp {
                return Err(invalid());
            }
            drop(named);
            let mappings = executable_mappings(&raw, &codes, kind.executable(), scan)?;
            let value = Self {
                pid,
                start,
                parent: capture.pid,
                parent_start: capture.start,
                pidfd,
                kind,
                command,
                codes,
                mappings,
            };
            value.recheck(proc, pg, capture, database, scan)?;
            Ok(value)
        }
        fn exited(&self, scan: &SharedJournalBudget) -> Result<bool> {
            // Root is not this helper's parent. A signalled original pidfd only
            // ends Root's live identity census duty; it supplies no exit code,
            // natural Child wait or M3 settlement/authority.
            pay(scan, 0, 2)?;
            require_cloexec(self.pidfd.fd.as_raw_fd())?;
            let mut poll = libc::pollfd {
                fd: self.pidfd.fd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let result = unsafe { libc::poll(&mut poll, 1, 0) };
            if result < 0 || poll.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
                return Err(invalid());
            }
            Ok(result == 1 && poll.revents & libc::POLLIN != 0)
        }
        fn recheck(
            &self,
            proc: &ProcView,
            pg: &PgCustody,
            capture: &MeasuredProcess,
            database: &str,
            scan: &SharedJournalBudget,
        ) -> Result<()> {
            fd_gate(4, scan)?;
            capture.recheck(proc, scan)?;
            if (capture.pid, capture.start) != (self.parent, self.parent_start)
                || actual_parent(proc, self.pid, self.start, scan)? != self.parent
            {
                return Err(invalid());
            }
            self.pidfd.revalidate(scan)?;
            helper_namespace(proc, self.pid, pg, capture, scan)?;
            let command = proc.read_relative(&format!("{}/cmdline", self.pid), 8192, scan)?;
            if command != self.command
                || original_pg_helper_recipe(&command, database, scan)? != self.kind
            {
                return Err(invalid());
            }
            original_tool_environment(proc, self.pid, scan)?;
            for code in &self.codes {
                code.recheck(scan)?;
            }
            let named = proc.open_checked_magic(&format!("{}/exe", self.pid), scan)?;
            let main = self
                .codes
                .iter()
                .find(|c| c.path == self.kind.executable())
                .ok_or_else(invalid)?;
            if Stamp::observe(&named, scan)? != main.stamp {
                return Err(invalid());
            }
            drop(named);
            if executable_mappings(
                &proc.read_relative(&format!("{}/maps", self.pid), 1024 * 1024, scan)?,
                &self.codes,
                &self.codes[0].path,
                scan,
            )? != self.mappings
            {
                return Err(invalid());
            }
            if actual_parent(proc, self.pid, self.start, scan)? != self.parent {
                return Err(invalid());
            }
            capture.recheck(proc, scan)
        }
    }

    const ENV_KEYS: [&[u8]; 7] = [
        b"PATH",
        b"HOME",
        b"LANG",
        b"PGHOST",
        b"PGPORT",
        b"PGUSER",
        b"PGDATABASE",
    ];
    const CONSTRUCTOR_QUERIES: [&str; 12] = [
        "PGPORT",
        "PGHOSTADDR",
        "PGHOST",
        "PGUSER",
        "PGDATABASE",
        "PGPASSWORD",
        "PGSSLROOTCERT",
        "PGSSLCERT",
        "PGSSLKEY",
        "PGSSLMODE",
        "PGAPPNAME",
        "PGOPTIONS",
    ];
    unsafe extern "C" {
        static mut environ: *mut *mut libc::c_char;
    }
    /// Caller must provide an actual C environment pointer array terminated at its
    /// eighth pointer at latest. Unknown key rejection never dereferences its value.
    pub(super) unsafe fn scan_environment(
        entries: *const *const libc::c_char,
        expected_host: &str,
        expected_db: &str,
        scan: &SharedJournalBudget,
    ) -> Result<()> {
        if entries.is_null() || !job::database(expected_db) {
            return Err(invalid());
        }
        let expected = [
            "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
            "/root",
            "C",
            expected_host,
            "5432",
            "learning_admin",
            expected_db,
        ];
        let mut seen = [false; 7];
        for index in 0..=7 {
            scan_charge(scan, std::mem::size_of::<*const libc::c_char>() as u64, 1)
                .map_err(|_| invalid())?;
            let entry = unsafe { *entries.add(index) };
            if entry.is_null() {
                return if index == 7 && seen.into_iter().all(|v| v) {
                    Ok(())
                } else {
                    Err(invalid())
                };
            }
            if index == 7 {
                return Err(invalid());
            }
            let mut key = [0_u8; 11];
            let mut length = None;
            for (offset, slot) in key.iter_mut().enumerate() {
                scan_charge(scan, 1, 0).map_err(|_| invalid())?;
                let byte = unsafe { *entry.add(offset) } as u8;
                if byte == b'=' {
                    length = Some(offset);
                    break;
                }
                if byte == 0 || offset == 10 {
                    return Err(invalid());
                }
                *slot = byte;
            }
            let length = length.ok_or(invalid())?;
            let slot = ENV_KEYS
                .iter()
                .position(|allowed| *allowed == &key[..length])
                .ok_or(invalid())?;
            if seen[slot] {
                return Err(invalid());
            }
            seen[slot] = true;
            // Only after matching the allowlisted key, inspect the exact fixed value.
            let value = expected[slot].as_bytes();
            for offset in 0..=value.len() {
                scan_charge(scan, 1, 0).map_err(|_| invalid())?;
                let byte = unsafe { *entry.add(length + 1 + offset) } as u8;
                if byte != value.get(offset).copied().unwrap_or(0) {
                    return Err(invalid());
                }
            }
        }
        Err(invalid())
    }

    struct CompilePins {
        manager_commit: &'static str,
        manager_build: &'static str,
        application_commit: &'static str,
        application_build: &'static str,
        source_auth: &'static str,
        verifier: &'static str,
        binding: &'static str,
    }
    impl CompilePins {
        fn read() -> Result<Self> {
            if option_env!("KNOWWEAVE_C4_B1_SOURCE_PROCESS_ROLE") != Some("root-launcher")
                || option_env!("KNOWWEAVE_C4_B1_ROLE") != Some("source")
                || option_env!("KNOWWEAVE_C4_B1_PROFILE") != Some(PROFILE)
            {
                return Err(invalid());
            }
            let pin = |v: Option<&'static str>, n| v.filter(|s| job::hex(s, n)).ok_or_else(invalid);
            Ok(Self {
                manager_commit: pin(option_env!("KNOWWEAVE_C4_B1_MANAGER_CODE_COMMIT"), 40)?,
                manager_build: pin(
                    option_env!("KNOWWEAVE_C4_B1_MANAGER_BUILD_INPUT_SHA256"),
                    64,
                )?,
                application_commit: pin(option_env!("KNOWWEAVE_SOURCE_COMMIT"), 40)?,
                application_build: pin(option_env!("KNOWWEAVE_BUILD_ID_SHA256"), 64)?,
                source_auth: pin(option_env!("KNOWWEAVE_C4_B1_SOURCE_AUTH_KEY_SHA256"), 64)?,
                verifier: pin(option_env!("KNOWWEAVE_C4_VERIFIER_KEY_SHA256"), 64)?,
                binding: pin(
                    option_env!("KNOWWEAVE_C4_SOURCE_CONTROL_BINDING_SHA256"),
                    64,
                )?,
            })
        }
        fn join(&self, c: &job::ExecutionCatalog, m: &job::LaunchManifest) -> Result<()> {
            let own = image(c, "RootLauncher")?;
            let s = &c.installed_source;
            if own.code_commit != self.manager_commit
                || own.build_input_sha256 != self.manager_build
                || s.application_commit != self.application_commit
                || s.application_build_sha256 != self.application_build
                || s.source_auth_public_key_sha256 != self.source_auth
                || s.source_control_binding_sha256 != self.binding
                || m.code_commit != self.manager_commit
                || m.build_input_sha256 != self.manager_build
                || m.binary_sha256 != own.executable.sha256
                || m.launcher_sha256 != own.executable.sha256
                || m.source_auth_public_key_sha256 != self.source_auth
                || m.destination_verifier_public_key_sha256 != self.verifier
                || m.launch_id != c.launch_id
                || c.capture_recipe_sha256 != c.installed_runtime.capture_mount_recipe_sha256
            {
                return Err(invalid());
            }
            Ok(())
        }
    }
    fn image<'a>(c: &'a job::ExecutionCatalog, role: &str) -> Result<&'a job::RoleImage> {
        let mut rows = c.images.iter().filter(|r| r.role == role);
        let row = rows.next().ok_or_else(invalid)?;
        if rows.next().is_some() {
            return Err(invalid());
        }
        Ok(row)
    }
    fn encode_hex(raw: &[u8]) -> String {
        hex::encode(raw)
    }
    struct HeldBlob {
        named: HeldNamed,
        raw: Vec<u8>,
        digest: [u8; 32],
    }
    impl HeldBlob {
        fn open(path: &str, cap: usize, mode: u32, scan: &SharedJournalBudget) -> Result<Self> {
            let mut named = HeldNamed::open(path, scan)?;
            if named.stamp.mode & 0o7777 != mode || named.stamp.gid != 0 {
                return Err(invalid());
            }
            let (raw, digest) = named.full_read_and_hash(cap as u64, scan)?;
            Ok(Self { named, raw, digest })
        }
        fn recheck(&mut self, scan: &SharedJournalBudget) -> Result<()> {
            let (raw, digest) = self.named.full_read_and_hash(self.raw.len() as u64, scan)?;
            if raw != self.raw || digest != self.digest {
                return Err(invalid());
            }
            Ok(())
        }
    }
    struct SourceOwner {
        child: Child,
        process: Option<MeasuredProcess>,
        control: Arc<UnixStream>,
        child_cookie: u64,
        waited: bool,
    }
    struct CaptureOwner {
        exec_id: String,
        transport: Option<Arc<UnixStream>>,
        attached: Option<AttachedOwner>,
        bootstrap: Option<(u32, u64)>,
        bootstrap_pidfd: Option<Arc<ParentPid>>,
        physical_shutdown: Option<Arc<PrepaidShutdown>>,
        negative_physical_drained: usize,
        process: Option<MeasuredProcess>,
        init: Option<(String, String, String)>,
        natural: bool,
    }
    struct Publication {
        parent: usize,
        name: String,
        file: Option<Arc<File>>,
        readback: Option<HeldBytesData>,
        raw: Vec<u8>,
        stamp: Option<super::Stamp>,
        durable: bool,
    }
    struct Generation {
        directory: usize,
        directory_identity: (u64, u64),
        reader_released: bool,
        lock: Option<File>,
        exclusive: bool,
        lock_stamp: Option<super::Stamp>,
        proof: usize,
        inspection: usize,
        binding: usize,
        phase: &'static str,
        request: String,
        observed: u64,
        deadline: Instant,
    }
    // Registry records here are pinned DATA observations, not Root admission.
    // Their first reads finish before proof issuance; fixed original directory
    // owners and full stamps/bytes remain. Rechecks never reconstruct a ctx.
    struct DataRecord {
        stamp: super::Stamp,
        raw: Vec<u8>,
        digest: [u8; 32],
    }
    impl DataRecord {
        fn retain_data(original: HeldBytesData) -> Self {
            Self {
                stamp: original.stamp,
                raw: original.raw,
                digest: original.digest,
            }
        }
        fn recheck(
            &self,
            parent: &BackupDir,
            leaf: &str,
            scan: &SharedJournalBudget,
        ) -> Result<()> {
            let now = HeldBytesData::open(parent, leaf, self.raw.len(), scan)?;
            pay(scan, self.raw.len(), 1)?;
            if now.stamp != self.stamp || now.raw != self.raw || now.digest != self.digest {
                return Err(invalid());
            }
            Ok(())
        }
    }
    struct HeldRoots {
        registry: BackupDir,
        authority: DataRecord,
        group: DataRecord,
        group_data: registry::SourceGroup,
        records: BTreeMap<String, registry::RootRecord>,
        record_bytes: Vec<DataRecord>,
        handles: BTreeMap<String, BackupDir>,
        host_paths: BTreeMap<String, String>,
        companion: Arc<BackupDir>,
        export: Arc<BackupDir>,
    }
    fn private_dir(path: &str, scan: &SharedJournalBudget) -> Result<BackupDir> {
        pay(scan, path.len() * 4 + 1024, 0)?;
        pay(scan, 0, path.split('/').count() * 5 + 4)?;
        fd_gate(2, scan)?;
        let d = BackupDir::open_trusted_private_root(Path::new(path))?;
        pay(scan, 0, 1)?;
        d.require_private_directory()?;
        Ok(d)
    }
    fn create_dir(parent: &BackupDir, leaf: &str, scan: &SharedJournalBudget) -> Result<BackupDir> {
        pay(scan, leaf.len() * 4 + 512, 6)?;
        fd_gate(2, scan)?;
        let value = parent.create_dir(leaf)?;
        parent.sync()?;
        value.require_private_directory()?;
        Ok(value)
    }
    fn roster(d: &BackupDir, cap: usize, scan: &SharedJournalBudget) -> Result<Vec<String>> {
        pay(scan, 65536 + (cap + 1) * 512 + 4096, (cap + 1) * 4 + 8)?;
        let mut names = d.list_bounded(cap)?;
        names.sort();
        Ok(names)
    }
    impl HeldRoots {
        fn open(
            host: &HostObservation,
            c: &job::ExecutionCatalog,
            scan: &SharedJournalBudget,
        ) -> Result<Self> {
            let registry_path = mapped(&host.current_mounts, registry::REGISTRY_PATH)?;
            let registry = private_dir(&registry_path, scan)?;
            let authority = HeldBytesData::open(&registry, "authority.json", 4096, scan)?;
            if hex::encode(authority.digest) != c.installed_source.registry_authority_sha256 {
                return Err(invalid());
            }
            let authority_data: registry::Authority = parse(&authority.raw, 4096, scan)?;
            if authority_data.deployment_id != c.installed_source.deployment_id
                || authority_data.registry_path != registry::REGISTRY_PATH
                || registry.identity()?
                    != (authority_data.registry_dev, authority_data.registry_ino)
            {
                return Err(invalid());
            }
            pay(scan, 1024, 8)?;
            let groups = registry.open_dir("groups")?;
            let group = HeldBytesData::open(
                &groups,
                &format!("{}.json", c.installed_source.source_group_id),
                4096,
                scan,
            )?;
            if hex::encode(group.digest) != c.installed_source.source_group_record_sha256 {
                return Err(invalid());
            }
            let group_data: registry::SourceGroup = parse(&group.raw, 4096, scan)?;
            if group_data.format_version != 1
                || group_data.capability != "backup_source_group_v1"
                || group_data.group_id != c.installed_source.source_group_id
                || group_data.deployment_id != c.installed_source.deployment_id
                || group_data.application_commit != c.installed_source.application_commit
                || group_data.application_build_sha256
                    != c.installed_source.application_build_sha256
                || group_data.database != c.installed_source.database
                || u64::from(group_data.database_oid) != c.installed_source.database_oid
                || group_data.system_identifier != c.installed_source.system_identifier
                || group_data.source_binding_sha256
                    != c.installed_source.source_control_binding_sha256
                || group_data
                    .roots
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    != ["assets", "control", "local_pins", "staging"]
            {
                return Err(invalid());
            }
            let root_parent = registry.open_dir("roots")?;
            let mut records = BTreeMap::new();
            let mut record_bytes = Vec::new();
            let mut handles = BTreeMap::new();
            let mut host_paths = BTreeMap::new();
            for (kind, id) in &group_data.roots {
                if !registry::v4(id) {
                    return Err(invalid());
                }
                let held = HeldBytesData::open(&root_parent, &format!("{id}.json"), 4096, scan)?;
                let record: registry::RootRecord = parse(&held.raw, 4096, scan)?;
                let expected = match kind.as_str() {
                    "assets" => "source_assets",
                    "control" => "source_control",
                    "local_pins" => "local_pins",
                    "staging" => "source_staging",
                    _ => return Err(invalid()),
                };
                if record.format_version != 1
                    || record.capability != "backup_root_v1"
                    || record.kind != expected
                    || record.enrollment_id != *id
                    || record.group_id != group_data.group_id
                    || record.deployment_id != group_data.deployment_id
                    || record.uid != 0
                    || record.mode != 0o700
                    || record.enrolled_generation == 0
                    || !registry::canonical_path(&record.path)
                {
                    return Err(invalid());
                }
                let host_path = mapped(&host.current_mounts, &record.path)?;
                let handle = private_dir(&host_path, scan)?;
                if handle.identity()? != (record.dev, record.ino) {
                    return Err(invalid());
                }
                pay(scan, 2048, 4)?;
                host_paths.insert(kind.clone(), host_path);
                handles.insert(kind.clone(), handle);
                records.insert(kind.clone(), record);
                record_bytes.push(DataRecord::retain_data(held));
            }
            if hash(&encode(&records, 16384, scan)?, scan)?
                != c.installed_runtime.source_root_records_sha256
            {
                return Err(invalid());
            }
            pay(
                scan,
                host.data.complete_container_projection.len() * 128,
                host.data.complete_container_projection.len(),
            )?;
            for container in &host.data.complete_container_projection {
                if container.container_id == c.installed_runtime.pg_container_id {
                    continue;
                }
                for mount in &container.mounts {
                    pay(scan, mount.source.len() * 4 + 128, 1)?;
                    if mount.rw
                        && host_paths
                            .values()
                            .chain(std::iter::once(&registry_path))
                            .any(|path| {
                                contains(path, &mount.source) || contains(&mount.source, path)
                            })
                    {
                        return Err(invalid());
                    }
                }
            }
            let companion = private_dir(&host.data.journal_mount, scan)?;
            let export = private_dir(&host.data.export_mount, scan)?;
            if companion.identity()? == export.identity()? {
                return Err(invalid());
            }
            Ok(Self {
                registry,
                authority: DataRecord::retain_data(authority),
                group: DataRecord::retain_data(group),
                group_data,
                records,
                record_bytes,
                handles,
                host_paths,
                companion: Arc::new(companion),
                export: Arc::new(export),
            })
        }
        fn recheck(&self, scan: &SharedJournalBudget) -> Result<()> {
            self.authority
                .recheck(&self.registry, "authority.json", scan)?;
            pay(scan, 2048, 8)?;
            let groups = self.registry.open_dir("groups")?;
            self.group
                .recheck(&groups, &format!("{}.json", self.group_data.group_id), scan)?;
            drop(groups);
            let roots = self.registry.open_dir("roots")?;
            for ((kind, record), bytes) in self.records.iter().zip(&self.record_bytes) {
                bytes.recheck(&roots, &format!("{}.json", record.enrollment_id), scan)?;
                let actual = private_dir(self.host_paths.get(kind).ok_or_else(invalid)?, scan)?;
                if actual.identity()? != self.handles[kind].identity()?
                    || actual.identity()? != (record.dev, record.ino)
                {
                    return Err(invalid());
                }
            }
            Ok(())
        }
    }
    fn read_original_publication(
        parent: &BackupDir,
        name: &str,
        file: Arc<File>,
        cap: usize,
        scan: &SharedJournalBudget,
    ) -> Result<HeldBytesData> {
        pay(scan, 0, 2)?;
        let m = file.metadata()?;
        if !m.is_file()
            || m.uid() != 0
            || m.gid() != 0
            || m.mode() & 0o7777 != 0o600
            || m.nlink() != 1
            || m.len() == 0
            || m.len() > cap as u64
        {
            return Err(invalid());
        }
        let stamp = super::Stamp::read(&m);
        let length = usize::try_from(m.len()).map_err(|_| invalid())?;
        pay(scan, length + 1, 2)?;
        let mut raw = vec![0; length];
        let mut position = file.as_ref();
        position.seek(SeekFrom::Start(0))?;
        file_exact_prepaid(&file, &mut raw, scan)?;
        let mut extra = [0];
        if file_read_prepaid(&file, &mut extra, scan)? != 0 {
            return Err(invalid());
        }
        let digest = digest_bytes(&raw, scan)?;
        let held = HeldBytesData {
            file,
            stamp,
            raw,
            digest,
            scan: scan.clone(),
        };
        held.recheck(parent, name, scan)?;
        Ok(held)
    }
    struct RootCapturedDataSummary {
        data: job::CapturedData,
        raw: Vec<u8>,
        digest: [u8; 32],
    }
    struct OriginalRootJob {
        scan: SharedJournalBudget,
        total: Instant,
        started_ns: u64,
        pins: CompilePins,
        native: OwnNativeView,
        own: MeasuredProcess,
        codes: Vec<Arc<MeasuredCode>>,
        catalog: FixedRecord<job::ExecutionCatalog>,
        manifest: FixedRecord<job::LaunchManifest>,
        close_slots: [Arc<std::sync::atomic::AtomicU8>; 4],
        source_shutdown: Option<PrepaidShutdown>,
        daemon_shutdown: Option<PrepaidShutdown>,
        daemon_negative_drained: usize,
        settlement_deadline: Option<Instant>,
        hold_durable: bool,
        failed_io: Arc<AtomicBool>,
        docker_build: Option<DockerBuild>,
        daemon_bootstrap: Option<Arc<UnixStream>>,
        http_effect_unknown: bool,
        pending_root_candidate: Option<(u32, u64)>,
        next_native_check: Instant,
        probe_events: i16,
        source_eof_seen: bool,
        capture_eof_seen: bool,
        http: Option<OriginalHttpOwner>,
        host_scan: Option<FixedHostScan>,
        host_ready: Option<HostObservation>,
        host_ready_started: Option<Instant>,
        io_deadline: Option<Instant>,
        sticky_negative: bool,
        host_paused: bool,
        reserved_three: Option<File>,
        directories: Vec<Arc<BackupDir>>,
        directory_paths: Vec<String>,
        jobs_owner: File,
        job_slot: usize,
        job_name: String,
        job_lock: File,
        job_lock_stamp: super::Stamp,
        ledger: RootControlLedger,
        publications: Vec<Publication>,
        source: Option<SourceOwner>,
        capture: Option<CaptureOwner>,
        docker: Option<FixedDocker>,
        pg: Option<PgCustody>,
        roots: Option<HeldRoots>,
        before: Option<HostObservation>,
        install: Option<Generation>,
        capture_generation: Option<Generation>,
        genesis: Option<HeldGenesisData>,
        runtime_probe: Option<RuntimeProbeOwner>,
        backup_slot: Option<usize>,
        source_publication: Option<usize>,
        capture_publication: Option<usize>,
        captured: Option<RootCapturedDataSummary>,
        tools: Vec<FixedTool>,
        native_helpers: Vec<OriginalPgHelper>,
        native_helper_codes: Vec<Arc<MeasuredCode>>,
        observation_bytes: u64,
        next_host_check: Instant,
        receiving: Option<RootChannel>,
        outbound_partial: [bool; 2],
        phase_deadline: Option<Instant>,
        negative_deadline: Option<Instant>,
        abort_attempted: [bool; 2],
        complete: bool,
        hold_attempted: bool,
    }
    extern "C" fn interrupted(_: libc::c_int) {
        STOP.store(true, Ordering::Release);
    }
    pub(super) fn run() -> Result<()> {
        // Burn before compile pins, clocks, environment, filesystem or FDs.
        ATTEMPT
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| invalid())?;
        let scan = SharedJournalBudget::default();
        let anchor = Instant::now();
        let started_ns = monotonic_ns(&scan)?;
        let total = anchor
            .checked_add(Duration::from_secs(1200))
            .ok_or_else(invalid)?;
        pay(&scan, 0, 1)?;
        if unsafe { libc::syscall(libc::SYS_close_range, 3_u32, u32::MAX, 4_u32) } != 0 {
            return Err(invalid());
        }
        let pins = CompilePins::read()?;
        pay(&scan, 0, 2)?;
        require_actual_root()?;
        // Reserve descriptor3 before Command creates its internal exec-error
        // pipe. A pre-existing descriptor3 is not adopted as a launcher input.
        pay(&scan, 0, 2)?;
        if unsafe { libc::fcntl(3, libc::F_GETFD) } >= 0
            || std::io::Error::last_os_error().raw_os_error() != Some(libc::EBADF)
        {
            return Err(invalid());
        }
        let raw = unsafe {
            libc::open(
                c"/dev/null".as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if raw != 3 {
            if raw >= 0 {
                unsafe { libc::close(raw) };
            }
            return Err(invalid());
        }
        let reserved_three = unsafe { File::from_raw_fd(raw) };
        let native = OwnNativeView::observe(&scan)?;
        native.proc.require_single_task(&scan)?;
        let init = TaskNamespace::observe(&native.proc, 1, &scan)?;
        if !native.task.object.same_object(&init.object) {
            return Err(invalid());
        }
        drop(init);
        let catalog = FixedRecord::<job::ExecutionCatalog>::open(CATALOG_PATH, 16384, &scan)?;
        let manifest = FixedRecord::<job::LaunchManifest>::open(
            "/var/lib/knowweave-c4/b1/launcher-launch-manifest.json",
            16384,
            &scan,
        )?;
        pins.join(&catalog.data, &manifest.data)?;
        let row = image(&catalog.data, "RootLauncher")?;
        if row.launch_manifest.bytes != manifest.raw.len() as u64
            || row.launch_manifest.sha256 != hex::encode(manifest.digest)
        {
            return Err(invalid());
        }
        unsafe {
            scan_environment(
                environ.cast(),
                "/run/knowweave-c4-source-pg",
                &catalog.data.installed_source.database,
                &scan,
            )?;
        }
        let mut codes = Vec::new();
        let own = MeasuredProcess::open(&native.proc, native.proc.own_pid, row, &mut codes, &scan)?;

        let mut random = [0_u8; 48];
        pay(&scan, 48, 1)?;
        if unsafe { libc::getrandom(random.as_mut_ptr().cast(), random.len(), 0) }
            != random.len() as isize
        {
            return Err(invalid());
        }
        let mut id: [u8; 16] = random[..16].try_into().map_err(|_| invalid())?;
        id[6] = (id[6] & 15) | 64;
        id[8] = (id[8] & 63) | 128;
        let job_name = Uuid::from_bytes(id).to_string();
        let cookie = hex::encode(&random[16..]);
        let observation = private_dir(OBSERVATION_ROOT, &scan)?;
        pay(&scan, 64, 4)?;
        let jobs = observation.open_dir("jobs")?;
        jobs.require_private_directory()?;
        let jobs_owner = open_beneath(
            &own.root,
            &format!("{OBSERVATION_ROOT}/jobs"),
            libc::O_RDONLY | libc::O_DIRECTORY,
            &scan,
        )?;
        pay(&scan, 0, 3)?;
        let jm = jobs_owner.metadata()?;
        if (jm.dev(), jm.ino()) != jobs.identity()?
            || unsafe { libc::flock(jobs_owner.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0
            || !roster(&jobs, 0, &scan)?.is_empty()
        {
            return Err(invalid());
        }
        let job_dir = Arc::new(create_dir(&jobs, &job_name, &scan)?);
        drop(observation);
        drop(jobs);
        pay(&scan, 64, 4)?;
        let job_lock = job_dir.create_file("job.lock")?;
        if unsafe { libc::flock(job_lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(invalid());
        }
        job_lock.sync_all()?;
        job_dir.sync()?;
        let job_lock_stamp = super::Stamp::read(&job_lock.metadata()?);
        pay(&scan, 640, 5)?;
        let close_slots: [Arc<std::sync::atomic::AtomicU8>; 4] =
            std::array::from_fn(|_| Arc::new(std::sync::atomic::AtomicU8::new(0)));
        let ledger = RootControlLedger::data_only(scan.clone(), &job_name, &cookie, total)?;
        pay(&scan, 0, 3)?;
        let mut action = unsafe { std::mem::zeroed::<libc::sigaction>() };
        action.sa_sigaction = interrupted as *const () as usize;
        unsafe { libc::sigemptyset(&mut action.sa_mask) };
        for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            if unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) } != 0 {
                return Err(invalid());
            }
        }
        let mut job = OriginalRootJob {
            scan,
            total,
            started_ns,
            pins,
            native,
            own,
            codes,
            catalog,
            manifest,
            close_slots,
            source_shutdown: None,
            daemon_shutdown: None,
            daemon_negative_drained: 0,
            settlement_deadline: None,
            hold_durable: false,
            failed_io: Arc::new(AtomicBool::new(false)),
            docker_build: None,
            daemon_bootstrap: None,
            http_effect_unknown: false,
            pending_root_candidate: None,
            next_native_check: anchor,
            probe_events: 0,
            source_eof_seen: false,
            capture_eof_seen: false,
            http: None,
            host_scan: None,
            host_ready: None,
            host_ready_started: None,
            io_deadline: None,
            sticky_negative: false,
            host_paused: false,
            reserved_three: Some(reserved_three),
            directories: vec![Arc::clone(&job_dir), Arc::clone(&job_dir), job_dir],
            directory_paths: vec![
                OBSERVATION_ROOT.into(),
                format!("{OBSERVATION_ROOT}/jobs"),
                format!("{OBSERVATION_ROOT}/jobs/{job_name}"),
            ],
            jobs_owner,
            job_slot: 2,
            job_name,
            job_lock,
            job_lock_stamp,
            ledger,
            publications: Vec::new(),
            source: None,
            capture: None,
            docker: None,
            pg: None,
            roots: None,
            before: None,
            install: None,
            capture_generation: None,
            genesis: None,
            runtime_probe: None,
            backup_slot: None,
            source_publication: None,
            capture_publication: None,
            captured: None,
            tools: Vec::new(),
            native_helpers: Vec::new(),
            native_helper_codes: Vec::new(),
            observation_bytes: 0,
            next_host_check: anchor,
            receiving: None,
            outbound_partial: [false; 2],
            phase_deadline: None,
            negative_deadline: None,
            abort_attempted: [false; 2],
            complete: false,
            hold_attempted: false,
        };
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job.execute()));
        match outcome {
            Ok(Ok(())) => {
                job.complete = true;
                Ok(())
            }
            _ => job.retain_hold(),
        }
    }
    #[derive(Serialize)]
    struct IsolationProof<'a> {
        format_version: u32,
        backup_id: &'a str,
        database: &'a str,
        compose_project: &'a str,
        observed_unix_ms: u64,
        driver_pid: u32,
        inspection_file: String,
        runtime_running: u32,
        worker_running: u32,
        other_admin_processes: u32,
        postgres_published_ports: u32,
        network_internal: bool,
        manager_runtime_secret_mounts: u32,
        manager_runtime_env_keys: u32,
        docker_inspection_sha256: String,
    }
    impl OriginalRootJob {
        fn open_original_docker(&mut self) -> Result<()> {
            if self.docker.is_some()
                || self.docker_build.is_some()
                || self.daemon_bootstrap.is_some()
                || self.http.is_some()
            {
                return Err(invalid());
            }
            self.boundary()?;
            pay(&self.scan, 4096, 8)?;
            self.docker_build = Some(DockerBuild {
                socket_file: None,
                socket_stamp: None,
                cli: None,
                cli_digest: None,
                daemon_pidfd: None,
                daemon_pid: 0,
                daemon_start: 0,
            });
            let build = self.docker_build.as_mut().ok_or_else(invalid)?;
            let raw = unsafe {
                libc::open(
                    c"/run/docker.sock".as_ptr(),
                    libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if raw < 0 {
                return Err(invalid());
            }
            build.socket_file = Some(unsafe { File::from_raw_fd(raw) });
            let stamp =
                Stamp::observe(build.socket_file.as_ref().ok_or_else(invalid)?, &self.scan)?;
            if stamp.uid != 0
                || stamp.mode & libc::S_IFMT != libc::S_IFSOCK
                || stamp.mode & 0o007 != 0
            {
                return Err(invalid());
            }
            build.socket_stamp = Some(stamp);
            build.cli = Some(HeldNamed::open("/usr/bin/docker", &self.scan)?);
            let (raw, digest) = build
                .cli
                .as_mut()
                .ok_or_else(invalid)?
                .full_read_and_hash(64 * 1024 * 1024, &self.scan)?;
            let digest = hex::encode(digest);
            if !raw.starts_with(b"\x7fELF")
                || option_env!("KNOWWEAVE_C4_DOCKER_CLI_SHA256") != Some(digest.as_str())
            {
                return Err(invalid());
            }
            build.cli_digest = Some(digest);
            self.critical()?;
            fd_gate(2, &self.scan)?;
            pay(&self.scan, 384, 4)?;
            let close_state = Arc::new(std::sync::atomic::AtomicU8::new(0));
            let raw = unsafe {
                libc::socket(
                    libc::AF_UNIX,
                    libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                    0,
                )
            };
            if raw < 0 {
                return Err(invalid());
            }
            self.daemon_bootstrap = Some(Arc::new(unsafe { UnixStream::from_raw_fd(raw) }));
            self.daemon_shutdown = Some(PrepaidShutdown::bind(
                Arc::clone(self.daemon_bootstrap.as_ref().ok_or_else(invalid)?),
                close_state,
            ));
            let mut address = unsafe { std::mem::zeroed::<libc::sockaddr_un>() };
            address.sun_family = libc::AF_UNIX as libc::sa_family_t;
            for (to, from) in address.sun_path.iter_mut().zip(DOCKER_SOCKET.as_bytes()) {
                *to = *from as libc::c_char;
            }
            let result = unsafe {
                libc::connect(
                    raw,
                    (&address as *const libc::sockaddr_un).cast(),
                    std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
                )
            };
            if result != 0
                && !matches!(
                    std::io::Error::last_os_error().raw_os_error(),
                    Some(libc::EINPROGRESS) | Some(libc::EAGAIN) | Some(libc::EINTR)
                )
            {
                return Err(invalid());
            }
            if result != 0 {
                loop {
                    self.cooperative_tick(self.operation_deadline())?;
                    pay(&self.scan, 0, 1)?;
                    let mut p = libc::pollfd {
                        fd: raw,
                        events: libc::POLLOUT,
                        revents: 0,
                    };
                    let n = unsafe { libc::poll(&mut p, 1, 0) };
                    if n < 0 {
                        if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
                        {
                            continue;
                        }
                        return Err(invalid());
                    }
                    if n == 0 {
                        continue;
                    }
                    if p.revents & (libc::POLLERR | libc::POLLNVAL | libc::POLLHUP) != 0 {
                        return Err(invalid());
                    }
                    break;
                }
            }
            let mut error = 0_i32;
            let mut size = std::mem::size_of_val(&error) as libc::socklen_t;
            pay(&self.scan, 0, 1)?;
            if unsafe {
                libc::getsockopt(
                    raw,
                    libc::SOL_SOCKET,
                    libc::SO_ERROR,
                    (&mut error as *mut i32).cast(),
                    &mut size,
                )
            } != 0
                || error != 0
            {
                return Err(invalid());
            }
            let (pid, _) = peer(
                self.daemon_bootstrap.as_ref().ok_or_else(invalid)?,
                &self.scan,
            )?;
            let start = actual_start(pid, &self.scan)?;
            let build = self.docker_build.as_mut().ok_or_else(invalid)?;
            build.daemon_pid = pid;
            build.daemon_start = start;
            pay(&self.scan, 128, 2)?;
            let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0_u32) };
            if raw < 0 {
                return Err(invalid());
            }
            build.daemon_pidfd = Some(ParentPid {
                fd: unsafe { OwnedFd::from_raw_fd(raw as i32) },
                pid,
                start,
            });
            build
                .daemon_pidfd
                .as_ref()
                .ok_or_else(invalid)?
                .revalidate(&self.scan)?;
            self.boundary()?;
            match self.docker_build.take() {
                Some(DockerBuild {
                    socket_file: Some(socket_file),
                    socket_stamp: Some(socket_stamp),
                    cli: Some(cli),
                    cli_digest: Some(cli_digest),
                    daemon_pidfd: Some(daemon_pidfd),
                    daemon_pid,
                    daemon_start,
                }) => {
                    self.docker = Some(FixedDocker {
                        socket_file,
                        socket_stamp,
                        cli,
                        cli_digest,
                        daemon_pidfd,
                        daemon_pid,
                        daemon_start,
                    });
                }
                other => {
                    self.docker_build = other;
                    return Err(invalid());
                }
            }
            // This connection sent no HTTP or daemon operation. Its finite
            // peer-measurement duty is complete; the original named socket,
            // daemon pidfd/start and CLI code owner now remain in the Book.
            self.daemon_shutdown = None;
            self.daemon_bootstrap = None;
            self.critical()?;
            self.boundary()
        }
        fn open_original_probe(&mut self, deadline: Instant) -> Result<()> {
            self.boundary()?;
            if self.http.is_some()
                || self.daemon_bootstrap.is_some()
                || self
                    .runtime_probe
                    .as_ref()
                    .is_none_or(|p| p.socket.is_some())
            {
                return Err(invalid());
            }
            let previous = self.enter_io_scope(deadline);
            fd_gate(2, &self.scan)?;
            pay(&self.scan, 384, 4)?;
            let close_state = Arc::new(std::sync::atomic::AtomicU8::new(0));
            let owner = self.runtime_probe.as_mut().ok_or_else(invalid)?;
            let raw = unsafe {
                libc::socket(
                    libc::AF_UNIX,
                    libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                    0,
                )
            };
            if raw < 0 {
                return Err(invalid());
            }
            owner.socket = Some(Arc::new(unsafe { UnixStream::from_raw_fd(raw) }));
            owner.shutdown = Some(PrepaidShutdown::bind(
                Arc::clone(owner.socket.as_ref().ok_or_else(invalid)?),
                close_state,
            ));
            self.probe_events = libc::POLLOUT;
            let mut address = unsafe { std::mem::zeroed::<libc::sockaddr_un>() };
            address.sun_family = libc::AF_UNIX as libc::sa_family_t;
            for (to, from) in address
                .sun_path
                .iter_mut()
                .zip(b"/run/knowweave-c4-source-pg/.s.PGSQL.5432")
            {
                *to = *from as libc::c_char;
            }
            let result = unsafe {
                libc::connect(
                    raw,
                    (&address as *const libc::sockaddr_un).cast(),
                    std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
                )
            };
            if result != 0
                && !matches!(
                    std::io::Error::last_os_error().raw_os_error(),
                    Some(libc::EINPROGRESS) | Some(libc::EAGAIN) | Some(libc::EINTR)
                )
            {
                return Err(invalid());
            }
            if result != 0 {
                loop {
                    self.cooperative_tick(deadline)?;
                    pay(&self.scan, 0, 1)?;
                    let mut p = libc::pollfd {
                        fd: raw,
                        events: libc::POLLOUT,
                        revents: 0,
                    };
                    let n = unsafe { libc::poll(&mut p, 1, 0) };
                    if n < 0 {
                        if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
                        {
                            continue;
                        }
                        return Err(invalid());
                    }
                    if n == 0 {
                        continue;
                    }
                    if p.revents & (libc::POLLERR | libc::POLLNVAL | libc::POLLHUP) != 0 {
                        return Err(invalid());
                    }
                    break;
                }
            }
            let mut error = 0_i32;
            let mut size = std::mem::size_of_val(&error) as libc::socklen_t;
            pay(&self.scan, 0, 1)?;
            if unsafe {
                libc::getsockopt(
                    raw,
                    libc::SOL_SOCKET,
                    libc::SO_ERROR,
                    (&mut error as *mut i32).cast(),
                    &mut size,
                )
            } != 0
                || error != 0
            {
                return Err(invalid());
            }
            self.probe_events = 0;
            self.finish_io_scope(previous)
        }
        fn host_barrier(&mut self) -> Result<()> {
            if self.docker.is_none() {
                return self.boundary();
            }
            let deadline = self.operation_deadline();
            if self.host_scan.is_none()
                && (self.host_ready_started.is_none() || Instant::now() >= self.next_host_check)
            {
                self.host_scan = Some(FixedHostScan::new(deadline, &self.scan)?);
            }
            while self.host_scan.is_some() {
                self.cooperative_tick(deadline)?;
            }
            self.critical()?;
            self.boundary()
        }
        fn pause_host_for_spawn(&mut self) -> Result<()> {
            if self.http.is_some()
                || self
                    .runtime_probe
                    .as_ref()
                    .is_some_and(|p| p.socket.is_some())
            {
                return Err(invalid());
            }
            while self.host_scan.is_some() {
                self.cooperative_tick(self.operation_deadline())?;
            }
            self.host_paused = true;
            self.boundary()
        }

        fn enter_io_scope(&mut self, deadline: Instant) -> Option<Instant> {
            let previous = self.io_deadline;
            self.io_deadline = Some(self.operation_deadline().min(deadline));
            previous
        }
        fn finish_io_scope(&mut self, previous: Option<Instant>) -> Result<()> {
            self.boundary()?;
            self.io_deadline = previous;
            self.boundary()
        }
        fn daemon_identity(&self) -> Result<(u32, u64)> {
            let daemon = self.docker.as_ref().ok_or_else(invalid)?;
            Ok((daemon.daemon_pid, daemon.daemon_start))
        }
        fn cooperative_tick(&mut self, deadline: Instant) -> Result<()> {
            let result = self.cooperative_tick_inner(deadline.min(self.operation_deadline()));
            if result.is_err() {
                self.sticky_negative = true;
                self.ledger.failed = true;
                self.failed_io.store(true, Ordering::Release);
            }
            result
        }
        fn cooperative_tick_inner(&mut self, deadline: Instant) -> Result<()> {
            root_io_boundary(deadline, &STOP)?;
            if self.sticky_negative && (!cleanup_io() || self.negative_deadline.is_none()) {
                return Err(invalid());
            }
            // Kernel/control negatives are tested every turn, including while
            // a partial HTTP/PG message is waiting. Expected data may stay
            // queued; early EOF/HUP or data in an idle phase is sticky.
            self.control_and_pidfd_tick()?;
            if Instant::now() >= self.next_native_check {
                self.critical()?;
                self.next_native_check = Instant::now()
                    .checked_add(Duration::from_millis(100))
                    .ok_or_else(invalid)?;
            }
            self.advance_original_tools()?;
            if self.negative_deadline.is_some() {
                self.advance_negative_readers()?;
            } else if !self.host_paused
                && self.docker.is_some()
                && (self.host_scan.is_some() || Instant::now() >= self.next_host_check)
            {
                let _scope = ObservationScope::enter();
                let before_bytes = self.scan.usage()?.charged_bytes;
                if self.host_scan.is_none() {
                    self.host_scan = Some(FixedHostScan::new(deadline, &self.scan)?);
                }
                let daemon = self.daemon_identity()?;
                let complete = self.host_scan.as_mut().ok_or_else(invalid)?.advance(
                    &self.catalog.data,
                    daemon,
                    deadline,
                    &self.scan,
                )?;
                self.check_partial_host_scan()?;
                if complete {
                    let state = self.host_scan.as_ref().ok_or_else(invalid)?;
                    let started = state.started;
                    let after = assemble_original_host(
                        &state.pack,
                        &self.catalog.data,
                        deadline,
                        &self.native.proc,
                        &self.docker.as_ref().ok_or_else(invalid)?.cli_digest,
                        &self.scan,
                    )?;
                    self.check_completed_host_scan(&after)?;
                    // Only the whole full-set scan, and its current native
                    // join, reaches ready. Partial rows never authorize work.
                    self.host_ready = Some(after);
                    self.host_ready_started = Some(started);
                    self.host_scan = None;
                    self.next_host_check = Instant::now()
                        .checked_add(Duration::from_secs(1))
                        .ok_or_else(invalid)?;
                }
                let charged = self
                    .scan
                    .usage()?
                    .charged_bytes
                    .checked_sub(before_bytes)
                    .ok_or_else(invalid)?;
                self.observation_bytes = self
                    .observation_bytes
                    .checked_add(charged)
                    .filter(|n| *n <= 64 * 1024 * 1024)
                    .ok_or_else(invalid)?;
            }
            // The fixed stack includes every owned wait surface. No new FD,
            // thread, callback, authority, quota or clock is created by poll.
            let mut fds = [libc::pollfd {
                fd: -1,
                events: 0,
                revents: 0,
            }; 16];
            let mut used = 0_usize;
            if let Some(http) = self.http.as_ref().filter(|h| !h.negative_eof) {
                fds[used] = libc::pollfd {
                    fd: http.socket.as_raw_fd(),
                    events: if self.negative_deadline.is_some() {
                        libc::POLLIN
                    } else {
                        http.events()
                    },
                    revents: 0,
                };
                used += 1;
            }
            if let Some(http) = self
                .host_scan
                .as_ref()
                .and_then(|h| h.http.as_ref())
                .filter(|h| !h.negative_eof)
            {
                fds[used] = libc::pollfd {
                    fd: http.socket.as_raw_fd(),
                    events: if self.negative_deadline.is_some() {
                        libc::POLLIN
                    } else {
                        http.events()
                    },
                    revents: 0,
                };
                used += 1;
            }
            if let Some(socket) = self
                .runtime_probe
                .as_ref()
                .filter(|p| !p.negative_eof)
                .and_then(|p| p.socket.as_ref())
            {
                fds[used] = libc::pollfd {
                    fd: socket.as_raw_fd(),
                    events: self.probe_events,
                    revents: 0,
                };
                used += 1;
            }
            if let Some(socket) = &self.daemon_bootstrap {
                fds[used] = libc::pollfd {
                    fd: socket.as_raw_fd(),
                    events: if self.negative_deadline.is_some() {
                        libc::POLLIN
                    } else {
                        libc::POLLOUT
                    },
                    revents: 0,
                };
                used += 1;
            }
            for channel in [RootChannel::Source, RootChannel::Capture] {
                let eof = if channel == RootChannel::Source {
                    self.source_eof_seen
                } else {
                    self.capture_eof_seen
                };
                if !eof && let Ok(fd) = self.control_fd(channel) {
                    fds[used] = libc::pollfd {
                        fd,
                        events: libc::POLLRDHUP,
                        revents: 0,
                    };
                    used += 1;
                }
            }
            let source = self
                .source
                .as_ref()
                .and_then(|s| s.process.as_ref())
                .and_then(|p| p.pidfd.as_deref());
            let capture = self.capture_pidfd();
            for (p, terminal) in [
                (source, self.ledger.step >= 16),
                (capture, self.ledger.step >= 15),
                (self.pg.as_ref().map(|p| &p.pidfd), false),
                (self.docker.as_ref().map(|d| &d.daemon_pidfd), false),
            ] {
                if let Some(p) = p
                    && !terminal
                {
                    fds[used] = libc::pollfd {
                        fd: p.fd.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    };
                    used += 1;
                }
            }
            for tool in &self.tools {
                for fd in [
                    tool.stdout
                        .as_ref()
                        .filter(|_| !tool.out_eof)
                        .map(AsRawFd::as_raw_fd),
                    tool.stderr
                        .as_ref()
                        .filter(|_| !tool.err_eof)
                        .map(AsRawFd::as_raw_fd),
                ]
                .into_iter()
                .flatten()
                {
                    if used == fds.len() {
                        return Err(invalid());
                    }
                    fds[used] = libc::pollfd {
                        fd,
                        events: libc::POLLIN,
                        revents: 0,
                    };
                    used += 1;
                }
            }
            pay(&self.scan, 0, 1)?;
            let ms = deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .min(25) as i32;
            let n = unsafe { libc::poll(fds.as_mut_ptr(), used as libc::nfds_t, ms) };
            if n < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                return Err(invalid());
            }
            root_io_boundary(deadline, &STOP)?;
            self.control_and_pidfd_tick()?;
            self.boundary()
        }
        fn control_and_pidfd_tick(&mut self) -> Result<()> {
            let negative = self.negative_deadline.is_some();
            for channel in [RootChannel::Source, RootChannel::Capture] {
                let Ok(fd) = self.control_fd(channel) else {
                    continue;
                };
                if if channel == RootChannel::Source {
                    self.source_eof_seen
                } else {
                    self.capture_eof_seen
                } {
                    continue;
                }
                pay(&self.scan, 0, 1)?;
                let mut p = libc::pollfd {
                    fd,
                    events: libc::POLLIN | libc::POLLRDHUP,
                    revents: 0,
                };
                let n = unsafe { libc::poll(&mut p, 1, 0) };
                if n < 0 {
                    return if std::io::Error::last_os_error().kind()
                        == std::io::ErrorKind::Interrupted
                    {
                        Ok(())
                    } else {
                        Err(invalid())
                    };
                }
                if p.revents & (libc::POLLNVAL | libc::POLLERR) != 0 {
                    return Err(invalid());
                }
                let terminal = if channel == RootChannel::Source {
                    self.ledger.step >= 16
                } else {
                    self.ledger.step >= 15
                };
                if !negative && !terminal && p.revents & (libc::POLLHUP | libc::POLLRDHUP) != 0 {
                    return Err(invalid());
                }
                self.ledger.wire.extra_probe(&self.scan)?;
                pay(&self.scan, 1, 1)?;
                let mut byte = 0_u8;
                let n = unsafe {
                    libc::recv(
                        fd,
                        (&mut byte as *mut u8).cast(),
                        1,
                        libc::MSG_PEEK | libc::MSG_DONTWAIT,
                    )
                };
                let expected = self.receiving == Some(channel)
                    || if channel == RootChannel::Source {
                        [1, 13].contains(&self.ledger.step)
                    } else {
                        [3, 5, 7, 9, 11].contains(&self.ledger.step)
                    };
                if n == 0 && !negative && !terminal || n > 0 && !negative && !expected {
                    return Err(invalid());
                }
                if n == 0 {
                    if channel == RootChannel::Source {
                        self.source_eof_seen = true
                    } else {
                        self.capture_eof_seen = true
                    }
                }
                if n < 0
                    && !matches!(
                        std::io::Error::last_os_error().kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    )
                {
                    return Err(invalid());
                }
            }
            let source = self
                .source
                .as_ref()
                .and_then(|s| s.process.as_ref())
                .and_then(|p| p.pidfd.as_deref());
            let capture = self.capture_pidfd();
            for (owner, terminal) in [
                (source, self.ledger.step >= 16),
                (capture, self.ledger.step >= 15),
                (self.pg.as_ref().map(|p| &p.pidfd), false),
                (self.docker.as_ref().map(|d| &d.daemon_pidfd), false),
            ] {
                if let Some(owner) = owner {
                    pay(&self.scan, 0, 1)?;
                    let mut p = libc::pollfd {
                        fd: owner.fd.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    };
                    let n = unsafe { libc::poll(&mut p, 1, 0) };
                    if n < 0
                        || p.revents & (libc::POLLERR | libc::POLLNVAL) != 0
                        || !negative && !terminal && p.revents & (libc::POLLIN | libc::POLLHUP) != 0
                    {
                        return Err(invalid());
                    }
                }
            }
            Ok(())
        }
        fn capture_pidfd(&self) -> Option<&ParentPid> {
            self.capture.as_ref().and_then(|c| {
                c.process
                    .as_ref()
                    .and_then(|p| p.pidfd.as_deref())
                    .or(c.bootstrap_pidfd.as_deref())
            })
        }
        fn advance_original_tools(&mut self) -> Result<()> {
            for tool in &mut self.tools {
                if tool.child.is_none() {
                    continue;
                }
                {
                    let child = tool.child.as_mut().ok_or_else(invalid)?;
                    if tool.stdout.is_none() {
                        tool.stdout = child.stdout.take();
                    }
                    if tool.stderr.is_none() {
                        tool.stderr = child.stderr.take();
                    }
                }
                if let Some(native) = &tool.native
                    && !native.exited(&self.scan)?
                {
                    native.recheck(
                        &self.native.proc,
                        &self.own,
                        tool.exe.as_ref().ok_or_else(invalid)?,
                        tool.stamp.ok_or_else(invalid)?,
                        tool.binding.as_ref().ok_or_else(invalid)?,
                        &self.scan,
                    )?;
                }
                for (which, fd) in [
                    (0, tool.stdout.as_ref().map(AsRawFd::as_raw_fd)),
                    (1, tool.stderr.as_ref().map(AsRawFd::as_raw_fd)),
                ] {
                    let Some(fd) = fd else {
                        continue;
                    };
                    if if which == 0 {
                        tool.out_eof
                    } else {
                        tool.err_eof
                    } {
                        continue;
                    }
                    let qualified = if which == 0 {
                        tool.out_nonblocking
                    } else {
                        tool.err_nonblocking
                    };
                    // A failed qualification leaves this exact pipe retained,
                    // but never permits a potentially blocking read in cleanup.
                    if !qualified {
                        continue;
                    }
                    require_original_tool_pipe(fd, &self.scan)?;
                    let mut bytes = [0_u8; 512];
                    pay(&self.scan, bytes.len(), 1)?;
                    let n = unsafe { libc::read(fd, bytes.as_mut_ptr().cast(), bytes.len()) };
                    if n == 0 {
                        if which == 0 {
                            tool.out_eof = true
                        } else {
                            tool.err_eof = true
                        }
                    } else if n > 0 {
                        let data = if which == 0 {
                            &mut tool.out
                        } else {
                            &mut tool.err
                        };
                        if data.len() + n as usize > 4096 {
                            return Err(invalid());
                        }
                        data.extend_from_slice(&bytes[..n as usize]);
                    } else if !matches!(
                        std::io::Error::last_os_error().kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) {
                        return Err(invalid());
                    }
                }
                if tool.status.is_none() {
                    pay(&self.scan, 0, 1)?;
                    tool.status = tool.child.as_mut().ok_or_else(invalid)?.try_wait()?;
                }
            }
            Ok(())
        }
        fn check_original_pg_processes(&mut self) -> Result<()> {
            let pg = self.pg.as_ref().ok_or_else(invalid)?;
            pg.recheck(&self.native.proc, &self.scan)?;
            let mut terminal_capture = None;
            if matches!(self.ledger.step, 15 | 16)
                && let Some(process) = self
                    .capture
                    .as_ref()
                    .filter(|capture| !capture.natural)
                    .and_then(|capture| capture.process.as_ref())
            {
                let original = process.pidfd.as_deref().ok_or_else(invalid)?;
                if (original.pid, original.start) != (process.pid, process.start) {
                    return Err(invalid());
                }
                pay(&self.scan, 0, 2)?;
                require_cloexec(original.fd.as_raw_fd())?;
                let mut poll = libc::pollfd {
                    fd: original.fd.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                let n = unsafe { libc::poll(&mut poll, 1, 0) };
                if n < 0 || poll.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
                    return Err(invalid());
                }
                if n == 1 && poll.revents & libc::POLLIN != 0 {
                    terminal_capture = Some((process.pid, process.start));
                }
            }
            let candidates =
                pg.native_root_candidates(&self.native.proc, terminal_capture, &self.scan)?;
            pay(
                &self.scan,
                candidates.len() * 128 + 1024,
                candidates.len() * 2 + 4,
            )?;
            // Only a kernel-signalled original pidfd ends an observer duty.
            // No helper exit code/Child settlement is supplied by this census.
            let mut index = 0;
            while index < self.native_helpers.len() {
                let helper = &self.native_helpers[index];
                if helper.exited(&self.scan)? {
                    if candidates.iter().any(|(pid, _, _)| *pid == helper.pid) {
                        return Err(invalid());
                    }
                    self.native_helpers.remove(index);
                } else {
                    if !candidates
                        .iter()
                        .any(|(pid, start, _)| (*pid, *start) == (helper.pid, helper.start))
                    {
                        return Err(invalid());
                    }
                    index += 1;
                }
            }
            let Some(capture) = self.capture.as_ref() else {
                return if candidates.is_empty() {
                    Ok(())
                } else {
                    Err(invalid())
                };
            };
            let row = image(&self.catalog.data, "Capture")?;
            let wrapper = &self.catalog.data.capture_exec_recipe.wrapper_code;
            let mut capture_seen = false;
            let mut helper_seen = 0_usize;
            for (pid, start, stamp) in candidates {
                let is_capture = capture
                    .process
                    .as_ref()
                    .is_some_and(|process| pid == process.pid && start == process.start)
                    || capture.bootstrap.is_some_and(|pair| pair == (pid, start))
                    || capture.process.is_none()
                        && capture.bootstrap.is_none()
                        && self.ledger.step == 2
                        && self
                            .pending_root_candidate
                            .is_none_or(|pair| pair == (pid, start));
                if is_capture {
                    if capture_seen
                        || capture.natural
                        || ![
                            (&row.executable.stamp, &row.executable.sha256),
                            (&wrapper.stamp, &wrapper.sha256),
                        ]
                        .into_iter()
                        .any(|(declared, digest)| {
                            stamp.same_declared(declared)
                                && self.codes.iter().any(|code| {
                                    code.stamp == stamp && hex::encode(code.digest) == *digest
                                })
                        })
                    {
                        return Err(invalid());
                    }
                    if let Some(process) = &capture.process {
                        if stamp != process.codes[0].stamp {
                            return Err(invalid());
                        }
                        process.recheck(&self.native.proc, &self.scan)?;
                    } else if capture.bootstrap.is_some() {
                        self.check_capture_bootstrap(pid, start)?;
                    }
                    if self
                        .pending_root_candidate
                        .is_some_and(|pair| pair != (pid, start))
                    {
                        return Err(invalid());
                    }
                    self.pending_root_candidate = Some((pid, start));
                    capture_seen = true;
                    continue;
                }
                if capture.natural {
                    return Err(invalid());
                }
                let original = capture.process.as_ref().ok_or_else(invalid)?;
                // B1OriginalHelper has a single retained Child slot. Siblings,
                // grandchildren, reparented tasks and concurrent extras fail.
                helper_seen += 1;
                if helper_seen > 1 {
                    return Err(invalid());
                }
                if let Some(helper) = self.native_helpers.iter().find(|helper| helper.pid == pid) {
                    if helper.start != start {
                        return Err(invalid());
                    }
                    helper.recheck(
                        &self.native.proc,
                        pg,
                        original,
                        &self.catalog.data.installed_source.database,
                        &self.scan,
                    )?;
                } else {
                    if !self.native_helpers.is_empty() {
                        return Err(invalid());
                    }
                    pay(&self.scan, std::mem::size_of::<OriginalPgHelper>(), 2)?;
                    let helper = OriginalPgHelper::open(
                        &self.native.proc,
                        (pid, start),
                        pg,
                        original,
                        &self.catalog.data.installed_source.database,
                        &mut self.native_helper_codes,
                        &self.scan,
                    )?;
                    self.native_helpers.push(helper);
                }
            }
            // Expected terminal absence is DATA only. The same original exec
            // inspect, physical doubleEOF and natural mux join still must all
            // pass in settle_capture before any CaptureSettlement is emitted.
            if capture.process.is_some()
                && !capture.natural
                && !capture_seen
                && terminal_capture.is_none()
            {
                return Err(invalid());
            }
            self.boundary()
        }
        fn check_partial_host_scan(&mut self) -> Result<()> {
            let Some(before) = &self.before else {
                return Ok(());
            };
            let scan = self.host_scan.as_mut().ok_or_else(invalid)?;
            if scan.pack.listed.is_some() && !scan.ids_checked {
                pay(
                    &self.scan,
                    scan.pack.ids.len() * 64 + 128,
                    scan.pack.ids.len(),
                )?;
                if scan.pack.ids.iter().map(String::as_str).ne(before
                    .data
                    .complete_container_projection
                    .iter()
                    .map(|v| v.container_id.as_str()))
                {
                    return Err(invalid());
                }
                scan.ids_checked = true;
            }
            // Rows are immutable in this round. Check only the most recent one
            // at its completion boundary, not an uncharged O(n^2) clone pass.
            while scan.checked_index < scan.index {
                let id = scan.pack.ids.get(scan.checked_index).ok_or_else(invalid)?;
                let raw = scan.pack.containers.get(id).ok_or_else(invalid)?;
                let row = projection(raw, &self.scan)?;
                if before
                    .data
                    .complete_container_projection
                    .iter()
                    .find(|v| v.container_id == *id)
                    != Some(&row)
                {
                    return Err(invalid());
                }
                scan.checked_index += 1;
            }
            if let Some(network) = &scan.pack.network
                && !scan.network_checked
            {
                if encode(network, RECORD_CAP, &self.scan)? != before.network {
                    return Err(invalid());
                }
                scan.network_checked = true;
            }
            if let Some(image) = &scan.pack.image
                && !scan.image_checked
            {
                if encode(image, RECORD_CAP, &self.scan)? != before.image {
                    return Err(invalid());
                }
                scan.image_checked = true;
            }
            Ok(())
        }
        fn check_completed_host_scan(&mut self, after: &HostObservation) -> Result<()> {
            if self.before.is_none() {
                return Ok(());
            }
            let before = self.before.as_ref().ok_or_else(invalid)?;
            let installed = self.installed_mounts()?;
            same_before_after(&before.data, &after.data, &installed, &self.scan)?;
            pay(
                &self.scan,
                before.network.len()
                    + after.network.len()
                    + before.image.len()
                    + after.image.len()
                    + before.security.len()
                    + after.security.len(),
                3,
            )?;
            if before.pg_pid != after.pg_pid
                || before.network != after.network
                || before.image != after.image
                || before.security != after.security
            {
                return Err(invalid());
            }
            self.check_original_pg_processes()
        }
        fn begin_foreground_http(
            &mut self,
            method: &str,
            path: &str,
            body: &[u8],
            upgrade: bool,
            cap: usize,
        ) -> Result<()> {
            self.boundary()?;
            if self.http.is_some()
                || self
                    .runtime_probe
                    .as_ref()
                    .is_some_and(|p| p.socket.is_some())
            {
                return Err(invalid());
            }
            if self.sticky_negative {
                let exec = &self.capture.as_ref().ok_or_else(invalid)?.exec_id;
                if !cleanup_io()
                    || method != "GET"
                    || path != format!("/v1.45/exec/{exec}/json")
                    || upgrade
                    || !body.is_empty()
                {
                    return Err(invalid());
                }
            }
            let d = self.docker.as_ref().ok_or_else(invalid)?;
            d.daemon_pidfd.revalidate(&self.scan)?;
            d.cli.revalidate(&self.scan)?;
            let purpose = if self.sticky_negative {
                HttpPurpose::NegativeExactExecInspect
            } else {
                HttpPurpose::Positive
            };
            let deadline = self.operation_deadline();
            self.http = Some(OriginalHttpOwner::new(
                (method, path, body),
                upgrade,
                cap,
                deadline,
                &self.scan,
                purpose,
            )?);
            loop {
                let daemon = self.daemon_identity()?;
                let deadline = self.operation_deadline();
                self.http
                    .as_mut()
                    .ok_or_else(invalid)?
                    .advance(daemon, deadline, &self.scan)?;
                self.cooperative_tick(deadline)?;
                let phase = self.http.as_ref().ok_or_else(invalid)?.phase;
                if matches!(phase, HttpPhase::Done | HttpPhase::Upgrade) {
                    break;
                }
            }
            self.boundary()
        }
        fn original_query(&mut self, path: &str, cap: usize) -> Result<serde_json::Value> {
            self.begin_foreground_http("GET", path, &[], false, cap)?;
            let owner = self.http.as_ref().ok_or_else(invalid)?;
            if owner.phase != HttpPhase::Done
                || owner.head.as_ref().ok_or_else(invalid)?.status != 200
            {
                return Err(invalid());
            }
            let value = projected_reply(path, &owner.body, cap, &self.scan)?;
            self.boundary()?;
            self.http = None;
            Ok(value)
        }
        fn original_create_capture(&mut self) -> Result<String> {
            #[derive(Serialize)]
            #[serde(rename_all = "PascalCase")]
            struct Create<'a> {
                attach_stdin: bool,
                attach_stdout: bool,
                attach_stderr: bool,
                tty: bool,
                privileged: bool,
                user: &'a str,
                cmd: Vec<&'a str>,
            }
            pay(&self.scan, 4096, 16)?;
            let recipe = &self.catalog.data.capture_exec_recipe;
            let cmd = std::iter::once(recipe.entrypoint.as_str())
                .chain(recipe.arguments.iter().map(String::as_str))
                .collect();
            let body = encode(
                &Create {
                    attach_stdin: true,
                    attach_stdout: true,
                    attach_stderr: true,
                    tty: false,
                    privileged: false,
                    user: "0:0",
                    cmd,
                },
                16384,
                &self.scan,
            )?;
            let path = format!(
                "/v1.45/containers/{}/exec",
                self.catalog.data.installed_runtime.pg_container_id
            );
            self.http_effect_unknown = true;
            self.begin_foreground_http("POST", &path, &body, false, 4096)?;
            let owner = self.http.as_ref().ok_or_else(invalid)?;
            if owner.phase != HttpPhase::Done
                || owner.head.as_ref().ok_or_else(invalid)?.status != 201
            {
                return Err(invalid());
            }
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Created {
                #[serde(rename = "Id")]
                id: String,
            }
            pay(&self.scan, owner.body.len() * 16, 4)?;
            let created: Created = serde_json::from_slice(&owner.body)?;
            if !crate::valid_digest(&created.id) {
                return Err(invalid());
            }
            self.boundary()?;
            self.http = None;
            Ok(created.id)
        }
        fn original_start_capture(&mut self) -> Result<()> {
            pay(&self.scan, 256, 1)?;
            let exec = self.capture.as_ref().ok_or_else(invalid)?.exec_id.clone();
            if !crate::valid_digest(&exec) {
                return Err(invalid());
            }
            self.begin_foreground_http(
                "POST",
                &format!("/v1.45/exec/{exec}/start"),
                br#"{"Detach":false,"Tty":false}"#,
                true,
                4096,
            )?;
            if self
                .http
                .as_ref()
                .is_none_or(|o| o.phase != HttpPhase::Upgrade)
            {
                return Err(invalid());
            }
            self.boundary()?;
            let physical = Arc::new(PrepaidShutdown::bind(
                Arc::clone(&self.http.as_ref().ok_or_else(invalid)?.socket),
                Arc::clone(&self.close_slots[3]),
            ));
            let capture = self.capture.as_mut().ok_or_else(invalid)?;
            let owner = self.http.take().ok_or_else(invalid)?;
            // No fallible operation between moving the exact hijacked socket
            // out of its completed HTTP duty and into the same Capture book.
            capture.transport = Some(owner.socket);
            capture.physical_shutdown = Some(physical);
            Ok(())
        }
        fn original_inspect_exec(&mut self) -> Result<(bool, Option<i64>, u32)> {
            pay(&self.scan, 256, 1)?;
            let exec = self.capture.as_ref().ok_or_else(invalid)?.exec_id.clone();
            if !crate::valid_digest(&exec) {
                return Err(invalid());
            }
            let value = self.original_query(&format!("/v1.45/exec/{exec}/json"), 16384)?;
            let catalog = &self.catalog.data;
            let config = &value["ProcessConfig"];
            if jstr(&value, "ID", 64)? != exec
                || jstr(&value, "ContainerID", 64)? != catalog.installed_runtime.pg_container_id
                || !jbool(&value, "OpenStdin")?
                || !jbool(&value, "OpenStdout")?
                || !jbool(&value, "OpenStderr")?
                || jstr(config, "entrypoint", 1024)? != catalog.capture_exec_recipe.entrypoint
                || jbool(config, "tty")?
                || jbool(config, "privileged")?
                || jstr(config, "user", 64)? != "0:0"
            {
                return Err(invalid());
            }
            pay(&self.scan, 16384, 16)?;
            let args: Vec<String> =
                serde_json::from_value(config.get("arguments").ok_or_else(invalid)?.clone())?;
            if args != catalog.capture_exec_recipe.arguments {
                return Err(invalid());
            }
            let pid = u32::try_from(ju64(&value, "Pid")?).map_err(|_| invalid())?;
            let (running, exit) = exec_exit_data(&value)?;
            self.boundary()?;
            Ok((running, exit, pid))
        }
        fn probe_write(&mut self, mut bytes: &[u8], deadline: Instant) -> Result<()> {
            let previous = self.enter_io_scope(deadline);
            let events = self.probe_events;
            self.probe_events = libc::POLLOUT;
            while !bytes.is_empty() {
                self.boundary()?;
                let socket = self
                    .runtime_probe
                    .as_ref()
                    .and_then(|p| p.socket.as_ref())
                    .ok_or_else(invalid)?;
                pay(&self.scan, bytes.len(), 1)?;
                let n = unsafe {
                    libc::send(
                        socket.as_raw_fd(),
                        bytes.as_ptr().cast(),
                        bytes.len(),
                        libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
                    )
                };
                if n > 0 {
                    bytes = &bytes[n as usize..];
                } else if n == 0
                    || !matches!(
                        std::io::Error::last_os_error().kind(),
                        std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                    )
                {
                    return Err(invalid());
                }
                self.cooperative_tick(deadline)?;
            }
            self.probe_events = events;
            self.finish_io_scope(previous)
        }
        fn probe_exact(&mut self, mut bytes: &mut [u8], deadline: Instant) -> Result<()> {
            let previous = self.enter_io_scope(deadline);
            let events = self.probe_events;
            self.probe_events = libc::POLLIN;
            while !bytes.is_empty() {
                self.boundary()?;
                let socket = self
                    .runtime_probe
                    .as_ref()
                    .and_then(|p| p.socket.as_ref())
                    .ok_or_else(invalid)?;
                pay(&self.scan, bytes.len(), 1)?;
                let n = unsafe {
                    libc::recv(
                        socket.as_raw_fd(),
                        bytes.as_mut_ptr().cast(),
                        bytes.len(),
                        libc::MSG_DONTWAIT,
                    )
                };
                if n > 0 {
                    bytes = &mut bytes[n as usize..];
                } else if n == 0
                    || !matches!(
                        std::io::Error::last_os_error().kind(),
                        std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                    )
                {
                    return Err(invalid());
                }
                self.cooperative_tick(deadline)?;
            }
            self.probe_events = events;
            self.finish_io_scope(previous)
        }
        fn probe_eof(&mut self, deadline: Instant) -> Result<()> {
            let previous = self.enter_io_scope(deadline);
            let events = self.probe_events;
            self.probe_events = libc::POLLIN;
            loop {
                self.boundary()?;
                let socket = self
                    .runtime_probe
                    .as_ref()
                    .and_then(|p| p.socket.as_ref())
                    .ok_or_else(invalid)?;
                pay(&self.scan, 1, 1)?;
                let mut byte = 0_u8;
                let n = unsafe {
                    libc::recv(
                        socket.as_raw_fd(),
                        (&mut byte as *mut u8).cast(),
                        1,
                        libc::MSG_DONTWAIT,
                    )
                };
                if n > 0 {
                    return Err(invalid());
                }
                if n == 0 {
                    break;
                }
                if !matches!(
                    std::io::Error::last_os_error().kind(),
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                ) {
                    return Err(invalid());
                }
                self.cooperative_tick(deadline)?;
            }
            self.probe_events = events;
            self.finish_io_scope(previous)
        }

        fn operation_deadline(&self) -> Instant {
            if let Some(negative) = self.negative_deadline {
                negative
                    .min(self.total)
                    .min(self.settlement_deadline.unwrap_or(self.total))
                    .min(self.phase_deadline.unwrap_or(self.total))
            } else {
                original_owner_deadline(
                    self.total,
                    self.ledger.deadline(),
                    self.phase_deadline,
                    None,
                )
                .min(self.io_deadline.unwrap_or(self.total))
            }
        }
        fn boundary(&self) -> Result<()> {
            if self.sticky_negative && (!cleanup_io() || self.negative_deadline.is_none()) {
                return Err(invalid());
            }
            root_io_boundary(self.operation_deadline(), &STOP)
        }
        fn bound_directory(&self, index: usize) -> Result<Arc<BackupDir>> {
            for g in [&self.install, &self.capture_generation]
                .into_iter()
                .flatten()
            {
                if g.directory == index && g.reader_released {
                    pay(&self.scan, 512, 4)?;
                    fd_gate(2, &self.scan)?;
                    let leaf = if g.phase == "Install" {
                        "install"
                    } else if g.phase == "Capture" {
                        "capture"
                    } else {
                        return Err(invalid());
                    };
                    let directory =
                        self.directories[self.backup_slot.ok_or_else(invalid)?].open_dir(leaf)?;
                    directory.require_private_directory()?;
                    if directory.identity()? != g.directory_identity {
                        return Err(invalid());
                    }
                    return Ok(Arc::new(directory));
                }
            }
            self.directories
                .get(index)
                .map(Arc::clone)
                .ok_or_else(invalid)
        }
        fn release_generation_reader(&mut self, install: bool) -> Result<()> {
            // Only the directory iterator duty ends. The exact exclusive lock
            // and every original created record FD remain in this same book.
            let g = if install {
                self.install.as_mut()
            } else {
                self.capture_generation.as_mut()
            }
            .ok_or_else(invalid)?;
            if g.reader_released {
                return Err(invalid());
            }
            g.reader_released = true;
            self.directories[g.directory] =
                Arc::clone(&self.directories[self.backup_slot.ok_or_else(invalid)?]);
            Ok(())
        }
        fn critical(&mut self) -> Result<()> {
            self.boundary()?;
            fd_gate(3, &self.scan)?;
            pay(&self.scan, 0, 4)?;
            require_actual_root()?;
            require_same_current_pid(self.own.pid, &self.scan)?;
            self.native.revalidate(&self.scan)?;
            self.own.recheck(&self.native.proc, &self.scan)?;
            self.pins.join(&self.catalog.data, &self.manifest.data)?;
            for code in self.codes.iter().chain(&self.native_helper_codes) {
                code.recheck(&self.scan)?;
            }
            self.catalog.recheck(&self.scan)?;
            self.manifest.recheck(&self.scan)?;
            if let Some(d) = &self.docker {
                d.daemon_pidfd.revalidate(&self.scan)?;
                d.cli.revalidate(&self.scan)?;
                pay(&self.scan, 128, 2)?;
                let named = open_beneath(&self.own.root, DOCKER_SOCKET, libc::O_PATH, &self.scan)?;
                if Stamp::observe(&d.socket_file, &self.scan)? != d.socket_stamp
                    || Stamp::observe(&named, &self.scan)? != d.socket_stamp
                {
                    return Err(invalid());
                }
            }
            unsafe {
                scan_environment(
                    environ.cast(),
                    "/run/knowweave-c4-source-pg",
                    &self.catalog.data.installed_source.database,
                    &self.scan,
                )?;
            }
            if let Some(source) = &self.source
                && !source.waited
                && self.ledger.step < 16
                && self.negative_deadline.is_none()
                && let Some(process) = &source.process
            {
                process.recheck(&self.native.proc, &self.scan)?;
                if !self.native.task.object.same_object(&process.user.object) {
                    return Err(invalid());
                }
            }
            if let Some(capture) = &self.capture
                && !capture.natural
                && self.ledger.step < 15
                && self.negative_deadline.is_none()
            {
                if let Some(process) = &capture.process {
                    process.recheck(&self.native.proc, &self.scan)?;
                    if !self.native.task.object.same_object(&process.user.object) {
                        return Err(invalid());
                    }
                } else if let Some((pid, start)) = capture.bootstrap {
                    self.check_capture_bootstrap(pid, start)?;
                }
            }
            if let Some(pg) = &self.pg {
                pg.recheck(&self.native.proc, &self.scan)?;
                if !self.native.task.object.same_object(&pg.user.object) {
                    return Err(invalid());
                }
            }
            if self.pg.is_some() && self.capture.is_some() && self.negative_deadline.is_none() {
                self.check_original_pg_processes()?;
            }
            if let Some(roots) = &self.roots {
                roots.recheck(&self.scan)?;
            }
            self.check_job_lock()?;
            if self
                .install
                .as_ref()
                .is_some_and(|g| g.binding != usize::MAX)
            {
                self.recheck_generation(
                    true,
                    self.publications
                        .iter()
                        .any(|p| p.name == OBSERVATION_NAME && p.durable),
                )?;
            }
            if self
                .capture_generation
                .as_ref()
                .is_some_and(|g| g.binding != usize::MAX)
            {
                self.recheck_generation(false, false)?;
            }
            fd_gate(1, &self.scan)?;
            self.boundary()
        }
        fn check_job_lock(&self) -> Result<()> {
            pay(&self.scan, 256, 8)?;
            let d = &self.directories[self.job_slot];
            d.require_private_directory()?;
            let named_job = open_beneath(
                &self.jobs_owner,
                &format!("/{}", self.job_name),
                libc::O_PATH | libc::O_DIRECTORY,
                &self.scan,
            )?;
            let named_identity = actual_object(&named_job, &self.scan)?;
            if (named_identity.dev, named_identity.ino) != d.identity()?
                || super::Stamp::read(&self.job_lock.metadata()?) != self.job_lock_stamp
                || super::Stamp::read(&d.open_file("job.lock")?.metadata()?) != self.job_lock_stamp
            {
                return Err(invalid());
            }
            // flock on the original file description must remain exclusive;
            // a fresh independent description must be unable to acquire it.
            let probe = d.open_file("job.lock")?;
            if unsafe { libc::flock(probe.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                unsafe { libc::flock(probe.as_raw_fd(), libc::LOCK_UN) };
                return Err(invalid());
            }
            if std::io::Error::last_os_error().raw_os_error() != Some(libc::EWOULDBLOCK) {
                return Err(invalid());
            }
            Ok(())
        }
        fn publish<T: Serialize>(
            &mut self,
            parent: usize,
            name: &str,
            value: &T,
            cap: usize,
        ) -> Result<usize> {
            self.boundary()?;
            let raw = encode(value, cap, &self.scan)?;
            self.publish_raw(parent, name, raw, cap)
        }
        fn publish_raw(
            &mut self,
            parent: usize,
            name: &str,
            raw: Vec<u8>,
            cap: usize,
        ) -> Result<usize> {
            if raw.is_empty() || raw.len() > cap {
                return Err(invalid());
            }
            if self.sticky_negative
                && (!cleanup_io() || parent != self.job_slot || name != "hold.json")
            {
                return Err(invalid());
            }
            self.boundary()?;
            pay(
                &self.scan,
                name.len() + std::mem::size_of::<Publication>() + 512,
                2,
            )?;
            let slot = self.publications.len();
            self.publications.push(Publication {
                parent,
                name: name.into(),
                file: None,
                readback: None,
                raw,
                stamp: None,
                durable: false,
            });
            fd_gate(3, &self.scan)?;
            pay(&self.scan, name.len() + 1, 2)?;
            let parent_file = open_beneath(
                &self.own.root,
                &self.directory_paths[parent],
                libc::O_RDONLY | libc::O_DIRECTORY,
                &self.scan,
            )?;
            let m = parent_file.metadata()?;
            if (m.dev(), m.ino()) != self.directories[parent].identity()?
                || m.uid() != 0
                || m.mode() & 0o7777 != 0o700
            {
                return Err(invalid());
            }
            let leaf = CString::new(name).map_err(|_| invalid())?;
            let fd = unsafe {
                libc::openat(
                    parent_file.as_raw_fd(),
                    leaf.as_ptr(),
                    libc::O_RDWR
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_NOFOLLOW
                        | libc::O_CLOEXEC,
                    0o600,
                )
            };
            if fd < 0 {
                return Err(invalid());
            }
            self.publications[slot].file = Some(Arc::new(unsafe { File::from_raw_fd(fd) }));
            drop(parent_file);
            let deadline = self.operation_deadline();
            let p = &mut self.publications[slot];
            let file = p.file.as_ref().ok_or_else(invalid)?;
            file_write_all_prepaid(file, &p.raw, deadline, &STOP, &self.scan)?;
            pay(&self.scan, 0, 2)?;
            let m = file.metadata()?;
            p.stamp = Some(super::Stamp::read(&m));
            if !m.is_file()
                || m.uid() != 0
                || m.gid() != 0
                || m.mode() & 0o7777 != 0o600
                || m.nlink() != 1
                || m.len() != p.raw.len() as u64
            {
                return Err(invalid());
            }
            file.sync_all()?;
            self.boundary()?;
            pay(&self.scan, 0, 1)?;
            self.directories[parent].sync()?;
            self.publications[slot].readback = Some(read_original_publication(
                &self.directories[parent],
                name,
                Arc::clone(self.publications[slot].file.as_ref().ok_or_else(invalid)?),
                cap,
                &self.scan,
            )?);
            self.recheck_publication(slot)?;
            self.publications[slot].durable = true;
            Ok(slot)
        }
        fn recheck_publication(&self, slot: usize) -> Result<()> {
            self.boundary()?;
            let p = self.publications.get(slot).ok_or_else(invalid)?;
            let read = p.readback.as_ref().ok_or_else(invalid)?;
            let stamp = p.stamp.as_ref().ok_or_else(invalid)?;
            pay(&self.scan, p.raw.len(), 1)?;
            if super::Stamp::read(&p.file.as_ref().ok_or_else(invalid)?.metadata()?) != *stamp
                || read.stamp != *stamp
                || p.raw != read.raw
            {
                return Err(invalid());
            }
            let parent = self.bound_directory(p.parent)?;
            read.recheck(&parent, &p.name, &self.scan)?;
            self.boundary()
        }
        fn publication_digest(&self, slot: usize) -> Result<String> {
            self.recheck_publication(slot)?;
            pay(&self.scan, 64, 0)?;
            Ok(hex::encode(
                self.publications[slot]
                    .readback
                    .as_ref()
                    .ok_or_else(invalid)?
                    .digest,
            ))
        }
        fn source_send(&mut self, data: RootControlData) -> Result<()> {
            self.send_original(RootChannel::Source, &data)
        }
        fn capture_send(&mut self, data: RootControlData) -> Result<()> {
            if matches!(data, RootControlData::BrokerInstallObserved { .. }) {
                return Err(invalid());
            }
            self.send_original(RootChannel::Capture, &data)
        }
        fn source_receive(&mut self) -> Result<RootControlData> {
            self.receive_original(RootChannel::Source)
        }
        fn capture_receive(&mut self) -> Result<RootControlData> {
            self.receive_original(RootChannel::Capture)
        }
        fn control_fd(&self, channel: RootChannel) -> Result<RawFd> {
            match channel {
                RootChannel::Source => Ok(self
                    .source
                    .as_ref()
                    .ok_or_else(invalid)?
                    .control
                    .as_raw_fd()),
                RootChannel::Capture => Ok(self
                    .capture
                    .as_ref()
                    .and_then(|c| c.attached.as_ref())
                    .ok_or_else(invalid)?
                    .control
                    .as_raw_fd()),
            }
        }
        fn supervise_waiting(&mut self) -> Result<()> {
            self.critical()?;
            let pid = self
                .capture
                .as_ref()
                .filter(|c| !c.natural)
                .and_then(|c| c.process.as_ref())
                .map(|p| p.pid);
            if let Some(pid) = pid {
                let (running, exit, actual) = self.original_inspect_exec()?;
                if !running || exit.is_some() || actual != pid {
                    return Err(invalid());
                }
            }
            self.cooperative_tick(self.operation_deadline())?;
            self.boundary()
        }
        fn poll_original(
            &mut self,
            channel: RootChannel,
            events: i16,
            deadline: Instant,
        ) -> Result<()> {
            root_io_boundary(deadline, &STOP)?;
            let fd = self.control_fd(channel)?;
            let mut list = [
                libc::pollfd {
                    fd,
                    events,
                    revents: 0,
                },
                libc::pollfd {
                    fd: -1,
                    events: 0,
                    revents: 0,
                },
                libc::pollfd {
                    fd: -1,
                    events: 0,
                    revents: 0,
                },
                libc::pollfd {
                    fd: -1,
                    events: 0,
                    revents: 0,
                },
            ];
            if let Some(process) = self.source.as_ref().and_then(|s| s.process.as_ref()) {
                list[1].fd = process.pidfd.as_ref().ok_or_else(invalid)?.fd.as_raw_fd();
                list[1].events = libc::POLLIN;
            }
            if let Some(process) = self.capture.as_ref().and_then(|c| c.process.as_ref()) {
                list[2].fd = process.pidfd.as_ref().ok_or_else(invalid)?.fd.as_raw_fd();
                list[2].events = libc::POLLIN;
            }
            if let Some(pg) = &self.pg {
                list[3].fd = pg.pidfd.fd.as_raw_fd();
                list[3].events = libc::POLLIN;
            }
            pay(&self.scan, 0, 1)?;
            let ms = deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .min(100) as i32;
            let n = unsafe { libc::poll(list.as_mut_ptr(), list.len() as libc::nfds_t, ms) };
            if n < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                return Err(invalid());
            }
            root_io_boundary(deadline, &STOP)?;
            if list
                .iter()
                .any(|p| p.revents & (libc::POLLNVAL | libc::POLLERR) != 0)
                || list
                    .iter()
                    .skip(1)
                    .any(|p| p.revents & (libc::POLLIN | libc::POLLHUP) != 0)
            {
                return Err(invalid());
            }
            self.supervise_waiting()?;
            root_io_boundary(deadline, &STOP)
        }
        fn read_original_bytes(
            &mut self,
            channel: RootChannel,
            mut bytes: &mut [u8],
            first: bool,
        ) -> Result<()> {
            let previous = self.enter_io_scope(self.operation_deadline());
            self.supervise_waiting()?;
            while !bytes.is_empty() {
                self.boundary()?;
                pay(&self.scan, bytes.len(), 1)?;
                let n = unsafe {
                    libc::recv(
                        self.control_fd(channel)?,
                        bytes.as_mut_ptr().cast(),
                        bytes.len(),
                        libc::MSG_DONTWAIT,
                    )
                };
                if n == 0 {
                    return Err(invalid());
                }
                if n > 0 {
                    if first && self.ledger.pending.is_none() {
                        let now = Instant::now();
                        self.ledger.pending = Some(
                            self.total.min(
                                now.checked_add(Duration::from_secs(30))
                                    .ok_or_else(invalid)?,
                            ),
                        );
                        if self.ledger.step == 5 && self.ledger.cutover.is_none() {
                            self.ledger.cutover = Some(
                                self.total.min(
                                    now.checked_add(Duration::from_secs(120))
                                        .ok_or_else(invalid)?,
                                ),
                            );
                        }
                    }
                    bytes = bytes.get_mut(n as usize..).ok_or_else(invalid)?;
                    continue;
                }
                match std::io::Error::last_os_error().kind() {
                    std::io::ErrorKind::Interrupted => self.supervise_waiting()?,
                    std::io::ErrorKind::WouldBlock => {
                        self.poll_original(channel, libc::POLLIN, self.operation_deadline())?
                    }
                    _ => return Err(invalid()),
                }
            }
            self.finish_io_scope(previous)
        }
        fn write_original_bytes(
            &mut self,
            channel: RootChannel,
            mut bytes: &[u8],
            deadline: Instant,
        ) -> Result<()> {
            let previous = self.enter_io_scope(deadline);
            self.supervise_waiting()?;
            while !bytes.is_empty() {
                root_io_boundary(deadline, &STOP)?;
                pay(&self.scan, bytes.len(), 1)?;
                let n = unsafe {
                    libc::send(
                        self.control_fd(channel)?,
                        bytes.as_ptr().cast(),
                        bytes.len(),
                        libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
                    )
                };
                if n == 0 {
                    return Err(invalid());
                }
                if n > 0 {
                    bytes = bytes.get(n as usize..).ok_or_else(invalid)?;
                    continue;
                }
                match std::io::Error::last_os_error().kind() {
                    std::io::ErrorKind::Interrupted => self.supervise_waiting()?,
                    std::io::ErrorKind::WouldBlock => {
                        self.poll_original(channel, libc::POLLOUT, deadline)?
                    }
                    _ => return Err(invalid()),
                }
            }
            root_io_boundary(deadline, &STOP)?;
            self.finish_io_scope(previous)
        }
        fn receive_original(&mut self, channel: RootChannel) -> Result<RootControlData> {
            self.ledger.begin()?;
            self.receiving = Some(channel);
            self.ledger.wire.prefix(&self.scan)?;
            let mut prefix = [0_u8; 4];
            self.read_original_bytes(channel, &mut prefix[..1], true)?;
            self.read_original_bytes(channel, &mut prefix[1..], false)?;
            let length = u32::from_be_bytes(prefix) as usize;
            self.ledger.wire.body(length, &self.scan)?;
            let mut raw = vec![0; length];
            self.read_original_bytes(channel, &mut raw, false)?;
            let value: RootControlData = parse(&raw, CONTROL_CAP, &self.scan)?;
            self.ledger
                .observe(&value, channel, RootDirection::Receive, Instant::now())?;
            self.boundary()?;
            self.receiving = None;
            self.ledger.failed = false;
            Ok(value)
        }
        fn send_original(&mut self, channel: RootChannel, value: &RootControlData) -> Result<()> {
            if matches!(value, RootControlData::BrokerInstallObserved { .. })
                && !self
                    .publications
                    .iter()
                    .any(|p| p.name == OBSERVATION_NAME && p.durable)
            {
                return Err(invalid());
            }
            self.host_barrier()?;
            self.ledger.begin()?;
            let deadline = self.operation_deadline();
            let raw = encode(value, CONTROL_CAP, &self.scan)?;
            self.ledger.wire.prefix(&self.scan)?;
            self.ledger.wire.body(raw.len(), &self.scan)?;
            self.ledger
                .observe(value, channel, RootDirection::Send, Instant::now())?;
            let slot = usize::from(channel == RootChannel::Capture);
            self.outbound_partial[slot] = true;
            let deadline = deadline.min(self.operation_deadline());
            self.write_original_bytes(channel, &(raw.len() as u32).to_be_bytes(), deadline)?;
            self.write_original_bytes(channel, &raw, deadline)?;
            self.outbound_partial[slot] = false;
            if channel == RootChannel::Capture {
                let a = self
                    .capture
                    .as_mut()
                    .and_then(|c| c.attached.as_mut())
                    .ok_or_else(invalid)?;
                a.expected_stdin = a
                    .expected_stdin
                    .checked_add(raw.len() + 4)
                    .ok_or_else(invalid)?;
            }
            self.ledger.failed = false;
            Ok(())
        }
        fn spawn_source(&mut self) -> Result<()> {
            self.critical()?;
            pay(&self.scan, 4096, 16)?;
            fd_gate(8, &self.scan)?;
            let (parent, child) = UnixStream::pair()?;
            mark_cloexec_nonblocking(parent.as_raw_fd())?;
            mark_cloexec_nonblocking(child.as_raw_fd())?;
            let parent = Arc::new(parent);
            self.source_shutdown = Some(PrepaidShutdown::bind(
                Arc::clone(&parent),
                Arc::clone(&self.close_slots[0]),
            ));
            let (_, child_cookie) = peer(&child, &self.scan)?;
            let endpoint = child.as_raw_fd();
            let mut command = Command::new("/opt/knowweave-c4/bin/knowweave-c4-b1-source");
            command
                .env_clear()
                .env(
                    "PATH",
                    "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
                )
                .env("HOME", "/root")
                .env("LANG", "C")
                .env("PGHOST", "/run/knowweave-c4-source-pg")
                .env("PGPORT", "5432")
                .env("PGUSER", "learning_admin")
                .env("PGDATABASE", &self.catalog.data.installed_source.database)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            // Only dup2/fcntl occur here. FD3 was reserved before Command's
            // exec-error pipe; unrelated original FDs stay CLOEXEC.
            unsafe {
                command.pre_exec(move || {
                    if libc::dup2(endpoint, 3) != 3 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if libc::fcntl(3, libc::F_SETFD, 0) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            pay(&self.scan, 0, 1)?;
            let child_process = command.spawn()?;
            // Register the actual Child owner before any fallible observation.
            self.source = Some(SourceOwner {
                child: child_process,
                process: None,
                control: parent,
                child_cookie,
                waited: false,
            });
            drop(child);
            self.reserved_three = None;
            let pid = self.source.as_ref().ok_or_else(invalid)?.child.id();
            let measured = MeasuredProcess::open(
                &self.native.proc,
                pid,
                image(&self.catalog.data, "PublicationSource")?,
                &mut self.codes,
                &self.scan,
            )?;
            if measured.namespace_pid != pid
                || !self.native.task.object.same_object(&measured.user.object)
            {
                return Err(invalid());
            }
            self.source.as_mut().ok_or_else(invalid)?.process = Some(measured);
            self.critical()?;
            let source = self.source.as_ref().ok_or_else(invalid)?;
            let start = job::SourceStart {
                schema_version: 1,
                job_id: self.job_name.clone(),
                launch_id: self.catalog.data.launch_id.clone(),
                catalog_sha256: hex::encode(self.catalog.digest),
                boot_id: self.native.sys.boot_id(&self.native.proc, &self.scan)?,
                root_cookie: self.ledger.cookie.clone(),
                capability: "b1_source_process_start_v1".into(),
                launcher: self.own.record(&self.scan)?,
                source: source
                    .process
                    .as_ref()
                    .ok_or_else(invalid)?
                    .record(&self.scan)?,
                source_control_fd: 3,
                source_socket_cookie: source.child_cookie,
                job_lock_dev: self.job_lock_stamp.dev,
                job_lock_ino: self.job_lock_stamp.ino,
                started_monotonic_ns: self.started_ns,
            };
            start.validate()?;
            let publication = self.publish(self.job_slot, "source-start.json", &start, 4096)?;
            self.source_publication = Some(publication);
            let digest = self.publication_digest(publication)?;
            self.source_send(RootControlData::SourceBootstrap {
                schema_version: 1,
                job_id: self.job_name.clone(),
                root_cookie: self.ledger.cookie.clone(),
                sequence: self.ledger.next[0][1],
                source_start_sha256: digest,
            })
        }
        fn start_capture(&mut self) -> Result<()> {
            self.critical()?;
            if !matches!(self.source_receive()?, RootControlData::SpawnCapture { .. }) {
                return Err(invalid());
            }
            if self.ledger.pending.is_none() {
                self.ledger.pending = Some(
                    self.total.min(
                        Instant::now()
                            .checked_add(Duration::from_secs(30))
                            .ok_or_else(invalid)?,
                    ),
                );
            }
            self.open_original_docker()?;
            let host = self.observe_host()?;
            let pg = PgCustody::open(
                &self.native.proc,
                host.pg_pid,
                &self.catalog.data,
                &self.scan,
            )?;
            if !self.native.task.object.same_object(&pg.user.object) {
                return Err(invalid());
            }
            pg.no_other_admin(&self.native.proc, 0, &self.scan)?;
            let roots = HeldRoots::open(&host, &self.catalog.data, &self.scan)?;
            if !roster(&roots.companion, 0, &self.scan)?.is_empty()
                || !roster(&roots.export, 0, &self.scan)?.is_empty()
            {
                return Err(invalid());
            }
            self.pg = Some(pg);
            self.roots = Some(roots);
            self.before = Some(host);
            self.measure_wrapper()?;
            let exec_id = self.original_create_capture()?;
            // Retain exact ID before start can create a live process.
            self.capture = Some(CaptureOwner {
                exec_id,
                transport: None,
                attached: None,
                bootstrap: None,
                bootstrap_pidfd: None,
                physical_shutdown: None,
                negative_physical_drained: 0,
                process: None,
                init: None,
                natural: false,
            });
            self.http_effect_unknown = false;
            self.original_start_capture()?;
            let owner = self.capture.as_mut().ok_or_else(invalid)?;
            let attached = Arc::clone(owner.transport.as_ref().ok_or_else(invalid)?);
            let physical = Arc::clone(owner.physical_shutdown.as_ref().ok_or_else(invalid)?);
            owner.attached = Some(AttachedOwner::start(
                attached,
                physical,
                Arc::clone(&self.close_slots[1]),
                Arc::clone(&self.close_slots[2]),
                Arc::clone(&self.failed_io),
                self.total,
                &self.scan,
            )?);
            // This is actual current exec/bootstrap DATA, not the final
            // Capture ProcessRef. The latter may only be measured after Init.
            let (running, exit, pid) = self.original_inspect_exec()?;
            if !running || exit.is_some() || pid == 0 {
                return Err(invalid());
            }
            let start = actual_start(pid, &self.scan)?;
            pay(&self.scan, 128, 3)?;
            let owner = self.capture.as_mut().ok_or_else(invalid)?;
            let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0_u32) };
            if raw < 0 {
                return Err(invalid());
            }
            owner.bootstrap_pidfd = Some(Arc::new(ParentPid {
                fd: unsafe { OwnedFd::from_raw_fd(raw as i32) },
                pid,
                start,
            }));
            owner.bootstrap = Some((pid, start));
            owner
                .bootstrap_pidfd
                .as_ref()
                .ok_or_else(invalid)?
                .revalidate(&self.scan)?;
            self.check_capture_bootstrap(pid, start)?;
            let source_start_sha256 =
                self.publication_digest(self.source_publication.ok_or_else(invalid)?)?;
            self.capture_send(RootControlData::CaptureBootstrap {
                schema_version: 1,
                job_id: self.job_name.clone(),
                root_cookie: self.ledger.cookie.clone(),
                sequence: self.ledger.next[1][1],
                source_start_sha256,
            })?;
            let (backup, nonce, request) = match self.capture_receive()? {
                RootControlData::CaptureInit {
                    backup_id,
                    capture_nonce,
                    request_id,
                    ..
                } => (backup_id, capture_nonce, request_id),
                _ => return Err(invalid()),
            };
            self.capture.as_mut().ok_or_else(invalid)?.init =
                Some((backup.clone(), nonce.clone(), request.clone()));
            let (running, _, pid) = self.original_inspect_exec()?;
            if !running || pid == 0 {
                return Err(invalid());
            }
            let original = self
                .capture
                .as_ref()
                .and_then(|c| c.bootstrap_pidfd.as_ref())
                .ok_or_else(invalid)?;
            let process = MeasuredProcess::open_with_original_pidfd(
                &self.native.proc,
                pid,
                image(&self.catalog.data, "Capture")?,
                &mut self.codes,
                &self.scan,
                Some(original),
            )?;
            if self.capture.as_ref().ok_or_else(invalid)?.bootstrap
                != Some((process.pid, process.start))
            {
                return Err(invalid());
            }
            if !self.native.task.object.same_object(&process.user.object) {
                return Err(invalid());
            }
            self.capture.as_mut().ok_or_else(invalid)?.process = Some(process);
            let pg = self.pg.as_ref().ok_or_else(invalid)?;
            let capture = self
                .capture
                .as_ref()
                .and_then(|c| c.process.as_ref())
                .ok_or_else(invalid)?;
            if capture.pid_name != pg.pid_name
                || capture.mnt_name != pg.mnt_name
                || !capture.user.object.same_object(&pg.user.object)
            {
                return Err(invalid());
            }
            let after = self.observe_host()?;
            self.same_host(&after)?;
            let backups = create_dir(&self.directories[self.job_slot], "backups", &self.scan)?;
            self.directories.push(Arc::new(backups));
            self.directory_paths
                .push(format!("{OBSERVATION_ROOT}/jobs/{}/backups", self.job_name));
            let backups_slot = self.directories.len() - 1;
            let directory = create_dir(&self.directories[backups_slot], &backup, &self.scan)?;
            self.directories.push(Arc::new(directory));
            self.directory_paths.push(format!(
                "{OBSERVATION_ROOT}/jobs/{}/backups/{backup}",
                self.job_name
            ));
            let parent = self.directories.len() - 1;
            self.backup_slot = Some(parent);
            // No writer or qualification is held by the intermediate backups
            // directory. Its only child was opened from the original job; keep
            // that original child and end this unused intermediate read duty.
            self.directories[backups_slot] = Arc::clone(&self.directories[self.job_slot]);
            self.critical()?;
            let capture = self.capture.as_ref().ok_or_else(invalid)?;
            let recipe = &self.catalog.data.capture_exec_recipe;
            let record = job::CaptureStart {
                schema_version: 1,
                job_id: self.job_name.clone(),
                launch_id: self.catalog.data.launch_id.clone(),
                catalog_sha256: hex::encode(self.catalog.digest),
                boot_id: self.native.sys.boot_id(&self.native.proc, &self.scan)?,
                root_cookie: self.ledger.cookie.clone(),
                capability: "b1_capture_process_start_v1".into(),
                backup_id: backup.clone(),
                capture_nonce: nonce,
                capture_request_id: request,
                source_start_sha256: self
                    .publication_digest(self.source_publication.ok_or_else(invalid)?)?,
                container_id: self.catalog.data.installed_runtime.pg_container_id.clone(),
                exec_id: capture.exec_id.clone(),
                image_id: self
                    .catalog
                    .data
                    .installed_runtime
                    .admitted_pg_image_id
                    .clone(),
                mount_recipe_sha256: self.catalog.data.capture_recipe_sha256.clone(),
                capture: capture
                    .process
                    .as_ref()
                    .ok_or_else(invalid)?
                    .record(&self.scan)?,
                native: self.pg.as_ref().ok_or_else(invalid)?.native(),
                image_ref: PG_IMAGE.into(),
                image_digest: PG_DIGEST.into(),
                exec_process_config: job::CaptureStartExecProcessConfig {
                    entrypoint: recipe.entrypoint.clone(),
                    arguments: recipe.arguments.clone(),
                    tty: false,
                    privileged: false,
                    user: "0:0".into(),
                },
                capture_exec_recipe_sha256: hash(&encode(recipe, 16384, &self.scan)?, &self.scan)?,
            };
            record.validate()?;
            let slot = self.publish(parent, "capture-start.json", &record, 16384)?;
            self.capture_publication = Some(slot);
            let digest = self.publication_digest(slot)?;
            self.capture_send(RootControlData::CaptureStartBound {
                schema_version: 1,
                job_id: self.job_name.clone(),
                root_cookie: self.ledger.cookie.clone(),
                sequence: self.ledger.next[1][1],
                backup_id: backup,
                capture_start_sha256: digest,
            })
        }
        fn measure_wrapper(&mut self) -> Result<()> {
            let pg = self.pg.as_ref().ok_or_else(invalid)?;
            let recipe = &self.catalog.data.capture_exec_recipe;
            let wrapper: job::CodeFile = parse(
                &encode(&recipe.wrapper_code, 4096, &self.scan)?,
                4096,
                &self.scan,
            )?;
            let capture = image(&self.catalog.data, "Capture")?;
            for code in std::iter::once(&wrapper)
                .chain(recipe.wrapper_libraries.iter())
                .chain(std::iter::once(&capture.executable))
                .chain(capture.libraries.iter())
            {
                if self.codes.iter().any(|c| {
                    c.stamp.same_declared(&code.stamp) && hex::encode(c.digest) == code.sha256
                }) {
                    continue;
                }
                if self.codes.len() >= 64 {
                    return Err(invalid());
                }
                self.codes.push(Arc::new(MeasuredCode::open(
                    Arc::clone(&pg.root),
                    code,
                    false,
                    &self.scan,
                )?));
            }
            Ok(())
        }
        fn observe_host(&mut self) -> Result<HostObservation> {
            self.boundary()?;
            let required = Instant::now();
            let deadline = self.operation_deadline();
            loop {
                if self.host_scan.is_none() && self.host_ready_started.is_none_or(|s| s < required)
                {
                    self.host_ready = None;
                    self.host_ready_started = None;
                    self.host_scan = Some(FixedHostScan::new(deadline, &self.scan)?);
                }
                self.cooperative_tick(deadline)?;
                if self.host_scan.is_none()
                    && self.host_ready_started.is_some_and(|s| s >= required)
                {
                    self.critical()?;
                    self.boundary()?;
                    return self.host_ready.take().ok_or_else(invalid);
                }
            }
        }
        fn same_host(&mut self, after: &HostObservation) -> Result<()> {
            let before = self.before.as_ref().ok_or_else(invalid)?;
            let installed = self.installed_mounts()?;
            same_before_after(&before.data, &after.data, &installed, &self.scan)?;
            pay(
                &self.scan,
                before.network.len()
                    + after.network.len()
                    + before.image.len()
                    + after.image.len()
                    + before.security.len()
                    + after.security.len(),
                3,
            )?;
            if before.pg_pid != after.pg_pid
                || before.network != after.network
                || before.image != after.image
                || before.security != after.security
            {
                return Err(invalid());
            }
            self.pg
                .as_ref()
                .ok_or_else(invalid)?
                .recheck(&self.native.proc, &self.scan)?;
            let capture = self.capture.as_ref().ok_or_else(invalid)?;
            let pid = capture_pid_for_phase(
                self.ledger.step,
                capture.natural,
                capture.process.as_ref().map(|p| p.pid),
                capture.bootstrap,
            )?;
            if !capture.natural && capture.process.is_none() {
                let (pid, start) = capture.bootstrap.ok_or_else(invalid)?;
                self.check_capture_bootstrap(pid, start)?;
            }
            let _ = pid;
            self.check_original_pg_processes()
        }
        fn check_capture_bootstrap(&self, pid: u32, start: u64) -> Result<()> {
            self.boundary()?;
            let pg = self.pg.as_ref().ok_or_else(invalid)?;
            if pid == 0
                || start == 0
                || actual_start(pid, &self.scan)? != start
                || process_status(&self.native.proc, pid, 0, &self.scan)? <= 1
            {
                return Err(invalid());
            }
            if self
                .pending_root_candidate
                .is_some_and(|p| p != (pid, start))
            {
                return Err(invalid());
            }
            self.capture_pidfd()
                .ok_or_else(invalid)?
                .revalidate(&self.scan)?;
            if self
                .native
                .proc
                .read_link_relative(&format!("{pid}/ns/pid"), &self.scan)?
                != pg.pid_name
                || self
                    .native
                    .proc
                    .read_link_relative(&format!("{pid}/ns/mnt"), &self.scan)?
                    != pg.mnt_name
            {
                return Err(invalid());
            }
            let root = self
                .native
                .proc
                .open_checked_magic(&format!("{pid}/root"), &self.scan)?;
            if actual_object(&root, &self.scan)? != pg.root_object {
                return Err(invalid());
            }
            drop(root);
            let task = TaskNamespace::observe(&self.native.proc, pid, &self.scan)?;
            if !task.object.same_object(&self.native.task.object) {
                return Err(invalid());
            }
            drop(task);
            let exe = self
                .native
                .proc
                .open_checked_magic(&format!("{pid}/exe"), &self.scan)?;
            let stamp = Stamp::observe(&exe, &self.scan)?;
            let row = image(&self.catalog.data, "Capture")?;
            let wrapper = &self.catalog.data.capture_exec_recipe.wrapper_code;
            if ![
                (&row.executable.stamp, &row.executable.sha256),
                (&wrapper.stamp, &wrapper.sha256),
            ]
            .into_iter()
            .any(|(declared, digest)| {
                stamp.same_declared(declared)
                    && self
                        .codes
                        .iter()
                        .any(|c| c.stamp == stamp && hex::encode(c.digest) == *digest)
            }) {
                return Err(invalid());
            }
            self.boundary()
        }
        fn installed_mounts(&self) -> Result<InstalledMountData> {
            pay(&self.scan, 2048, 1)?;
            let r = &self.catalog.data.installed_runtime;
            Ok(InstalledMountData {
                container_id: r.pg_container_id.clone(),
                image_sha256: r.admitted_pg_image_id.clone(),
                daemon_id: r.daemon_id.clone(),
                compose_project: r.compose_project.clone(),
                assets_inside: self.roots.as_ref().ok_or_else(invalid)?.records["assets"]
                    .path
                    .clone(),
                mount_digest: r.capture_mount_recipe_sha256.clone(),
            })
        }
        fn pg_dump_version(&mut self) -> Result<(String, String)> {
            let pg = self.pg.as_ref().ok_or_else(invalid)?;
            let binding = CodeBinding::open(
                Arc::clone(&pg.root),
                "/usr/lib/postgresql/18/bin/pg_dump",
                &self.scan,
            )?;
            let mut exe = binding.open_leaf(&self.scan)?;
            let stamp = Stamp::observe(&exe, &self.scan)?;
            if !stamp.root_file(64 * 1024 * 1024) {
                return Err(invalid());
            }
            let bytes = read_exact_file(&mut exe, stamp.bytes, &self.scan)?;
            if !bytes.starts_with(b"\x7fELF") {
                return Err(invalid());
            }
            let digest = hex::encode(digest_bytes(&bytes, &self.scan)?);
            self.pause_host_for_spawn()?;
            pay(&self.scan, 8192, 16)?;
            fd_gate(8, &self.scan)?;
            // Only this literal read-only tool action is possible. It executes
            // the actual image ELF through our original held PG task root;
            // this true Child wait is never used as capture-exec settlement.
            let fd = exe.as_raw_fd();
            let mut command = Command::new("/usr/lib/postgresql/18/bin/pg_dump");
            command
                .arg("--version")
                .env_clear()
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            // Run that exact original opened ELF, even if a named path is
            // concurrently replaced. All child-side values are fixed literals;
            // execveat and errno reporting are the only pre_exec operations.
            unsafe {
                command.pre_exec(move || {
                    let argv = [c"pg_dump".as_ptr(), c"--version".as_ptr(), std::ptr::null()];
                    let env = [
                        c"PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
                            .as_ptr(),
                        c"HOME=/root".as_ptr(),
                        c"LANG=C".as_ptr(),
                        std::ptr::null(),
                    ];
                    libc::syscall(
                        libc::SYS_execveat,
                        fd,
                        c"".as_ptr(),
                        argv.as_ptr(),
                        env.as_ptr(),
                        libc::AT_EMPTY_PATH,
                    );
                    Err(std::io::Error::last_os_error())
                });
            }
            let child = command.spawn()?;
            self.tools.push(FixedTool {
                child: Some(child),
                stdout: None,
                stderr: None,
                out: Vec::with_capacity(4097),
                err: Vec::with_capacity(4097),
                out_eof: false,
                err_eof: false,
                out_nonblocking: false,
                err_nonblocking: false,
                status: None,
                native: None,
                exe: Some(exe),
                binding: Some(binding),
                stamp: Some(stamp),
            });
            let index = self.tools.len() - 1;
            self.host_paused = false;
            let tool = &mut self.tools[index];
            let child = tool.child.as_mut().ok_or_else(invalid)?;
            tool.stdout = child.stdout.take();
            tool.stderr = child.stderr.take();
            qualify_original_tool_pipe(
                tool.stdout.as_ref().ok_or_else(invalid)?.as_raw_fd(),
                &self.scan,
            )?;
            tool.out_nonblocking = true;
            qualify_original_tool_pipe(
                tool.stderr.as_ref().ok_or_else(invalid)?.as_raw_fd(),
                &self.scan,
            )?;
            tool.err_nonblocking = true;
            // Never accept zero exit/stdout from an unobserved host helper.
            // A too-fast/unobservable child remains a closed/unknown failure.
            let tool = &self.tools[index];
            let pid = tool.child.as_ref().ok_or_else(invalid)?.id();
            let native = RootVersionNative::open(
                &self.native.proc,
                pid,
                &self.own,
                tool.exe.as_ref().ok_or_else(invalid)?,
                tool.stamp.ok_or_else(invalid)?,
                tool.binding.as_ref().ok_or_else(invalid)?,
                &self.scan,
            )?;
            self.tools[index].native = Some(native);
            let deadline = self.operation_deadline();
            loop {
                self.cooperative_tick(deadline)?;
                let tool = &self.tools[index];
                if tool.status.is_some() && tool.out_eof && tool.err_eof {
                    break;
                }
            }
            self.boundary()?;
            let tool = &mut self.tools[index];
            if tool
                .status
                .as_ref()
                .and_then(std::process::ExitStatus::code)
                != Some(0)
                || !tool.err.is_empty()
                || Stamp::observe(tool.exe.as_ref().ok_or_else(invalid)?, &self.scan)?
                    != tool.stamp.ok_or_else(invalid)?
                || !tool
                    .native
                    .as_ref()
                    .ok_or_else(invalid)?
                    .exited(&self.scan)?
            {
                return Err(invalid());
            }
            if tool.native.as_ref().ok_or_else(invalid)?.pid
                != tool.child.as_ref().ok_or_else(invalid)?.id()
            {
                return Err(invalid());
            }
            pay(&self.scan, tool.out.len() * 2 + 256, 1)?;
            let version = std::str::from_utf8(&tool.out)
                .map_err(|_| invalid())?
                .strip_suffix('\n')
                .ok_or_else(invalid)?;
            if !version.starts_with("pg_dump (PostgreSQL) 18.6 (Debian 18.6-")
                || version.len() > 128
                || version.contains(['\n', '\r'])
            {
                return Err(invalid());
            }
            let result = version.to_owned();
            // Natural true-child wait plus real pipe EOF have settled this
            // helper. These tool-only handles, including any stdlib child
            // process handle, finish their duties after that observed wait.
            // All admitted role/wrapper code FDs in self.codes remain held.
            tool.stdout = None;
            tool.stderr = None;
            tool.native = None;
            tool.exe = None;
            tool.binding = None;
            tool.child = None;
            Ok((digest, result))
        }
        fn issue_generation(&mut self, install: bool) -> Result<()> {
            self.critical()?;
            let request = match self.capture_receive()? {
                RootControlData::RequestInstallProof { request_id, .. } if install => request_id,
                RootControlData::RequestCaptureProof { request_id, .. } if !install => request_id,
                _ => return Err(invalid()),
            };
            let after = self.observe_host()?;
            self.same_host(&after)?;
            self.check_original_pg_processes()?;
            let (phase, leaf) = if install {
                ("Install", "install")
            } else {
                ("Capture", "capture")
            };
            let backup = self.ledger.backup.as_ref().ok_or_else(invalid)?.clone();
            let parent = self.backup_slot.ok_or_else(invalid)?;
            let directory = create_dir(&self.directories[parent], leaf, &self.scan)?;
            self.directories.push(Arc::new(directory));
            self.directory_paths
                .push(format!("{}/{leaf}", self.directory_paths[parent]));
            let dir = self.directories.len() - 1;
            pay(&self.scan, 512, 8)?;
            let lock_name = format!("isolation-{backup}.lock");
            let generation = Generation {
                directory: dir,
                directory_identity: self.directories[dir].identity()?,
                reader_released: false,
                lock: None,
                exclusive: false,
                lock_stamp: None,
                proof: usize::MAX,
                inspection: usize::MAX,
                binding: usize::MAX,
                phase,
                request: request.clone(),
                observed: 0,
                deadline: self.operation_deadline(),
            };
            if install {
                if self.install.is_some() {
                    return Err(invalid());
                }
                self.install = Some(generation)
            } else {
                if self.capture_generation.is_some() {
                    return Err(invalid());
                }
                self.capture_generation = Some(generation)
            }
            // The stable original book exists before create. The returned File
            // moves straight into that slot before flock/sync/stat can fail.
            // A partial acquire is never dropped while a consumer is unknown.
            let generation = if install {
                self.install.as_mut()
            } else {
                self.capture_generation.as_mut()
            }
            .ok_or_else(invalid)?;
            generation.lock = Some(self.directories[dir].create_file(&lock_name)?);
            let lock = generation.lock.as_ref().ok_or_else(invalid)?;
            if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err(invalid());
            }
            generation.exclusive = true;
            lock.sync_all()?;
            self.directories[dir].sync()?;
            generation.lock_stamp = Some(super::Stamp::read(&lock.metadata()?));
            let (pg_dump_sha256, pg_dump_version) = self.pg_dump_version()?;
            self.critical()?;
            let pg = self.pg.as_ref().ok_or_else(invalid)?;
            let r = &self.catalog.data.installed_runtime;
            pay(&self.scan, 16384, 64)?;
            let containers=after.data.complete_container_projection.iter().filter(|c|c.project.as_deref()==Some(&r.compose_project)).map(|c|serde_json::json!({"service":c.service,"running":c.running,"container_id":c.container_id})).collect::<Vec<_>>();
            let native = serde_json::json!({"pid_namespace":pg.pid_name,"mount_namespace":pg.mnt_name,"postmaster_start_ticks":pg.start,"data_dev":pg.data_object.dev,"data_ino":pg.data_object.ino,"socket_dev":pg.socket_object.dev,"socket_ino":pg.socket_object.ino});
            let inspection = serde_json::json!({"network":{"project":r.compose_project,"internal":true},"containers":containers,"runtime_running":0,"worker_running":0,"other_admin_processes":0,"postgres_published_ports":0,"network_internal":true,"manager_runtime_secret_mounts":0,"manager_runtime_env_keys":0,"source_endpoint":{"format_version":1,"capability":"source_native_endpoint_v1","epoch":backup,"project":r.compose_project,"container_id":r.pg_container_id,"image_id":r.admitted_pg_image_id,"image_digest":r.pg_image_digest,"daemon_id":r.daemon_id,"network_id":r.internal_network_id,"mounts_sha256":r.capture_mount_recipe_sha256,"pg_dump_sha256":pg_dump_sha256,"pg_dump_version":pg_dump_version,"native":native}});
            let observed = now_ms(&self.scan)?;
            let proof = IsolationProof {
                format_version: 2,
                backup_id: &backup,
                database: &self.catalog.data.installed_source.database,
                compose_project: &r.compose_project,
                observed_unix_ms: observed,
                driver_pid: self.own.pid,
                inspection_file: format!("inspection-{backup}.json"),
                runtime_running: 0,
                worker_running: 0,
                other_admin_processes: 0,
                postgres_published_ports: 0,
                network_internal: true,
                manager_runtime_secret_mounts: 0,
                manager_runtime_env_keys: 0,
                docker_inspection_sha256: hash(
                    &encode(&inspection, RECORD_CAP, &self.scan)?,
                    &self.scan,
                )?,
            };
            let proof_raw = encode(&proof, 4096, &self.scan)?;
            let inspection_slot = self.publish(
                dir,
                &format!("inspection-{backup}.json"),
                &inspection,
                RECORD_CAP,
            )?;
            let proof_slot =
                self.publish_raw(dir, &format!("isolation-{backup}.json"), proof_raw, 4096)?;
            let g = if install {
                self.install.as_ref()
            } else {
                self.capture_generation.as_ref()
            }
            .ok_or_else(invalid)?;
            if !g.exclusive {
                return Err(invalid());
            }
            let lock_stamp = g.lock_stamp.as_ref().ok_or_else(invalid)?;
            let binding = job::GenerationBinding {
                schema_version: 1,
                capability: "b1_immutable_isolation_generation_v1".into(),
                job_id: self.job_name.clone(),
                backup_id: backup.clone(),
                phase: phase.into(),
                request_id: request.clone(),
                capture_start_sha256: self
                    .publication_digest(self.capture_publication.ok_or_else(invalid)?)?,
                isolation_bytes: self.publications[proof_slot].raw.len() as u64,
                isolation_sha256: self.publication_digest(proof_slot)?,
                inspection_bytes: self.publications[inspection_slot].raw.len() as u64,
                inspection_sha256: self.publication_digest(inspection_slot)?,
                lock_dev: lock_stamp.dev,
                lock_ino: lock_stamp.ino,
                observed_unix_ms: observed,
            };
            binding.validate()?;
            let binding_slot = self.publish(dir, "generation-binding.json", &binding, 4096)?;
            let digest = self.publication_digest(binding_slot)?;
            let g = if install {
                self.install.as_mut()
            } else {
                self.capture_generation.as_mut()
            }
            .ok_or_else(invalid)?;
            g.proof = proof_slot;
            g.inspection = inspection_slot;
            g.binding = binding_slot;
            g.observed = observed;
            self.critical()?;
            if now_ms(&self.scan)?
                .checked_sub(observed)
                .is_none_or(|age| age > 30_000)
            {
                return Err(invalid());
            }
            let reply = if install {
                RootControlData::InstallProofBound {
                    schema_version: 1,
                    job_id: self.job_name.clone(),
                    root_cookie: self.ledger.cookie.clone(),
                    sequence: self.ledger.next[1][1],
                    backup_id: backup,
                    request_id: request,
                    generation_binding_sha256: digest,
                }
            } else {
                RootControlData::CaptureProofBound {
                    schema_version: 1,
                    job_id: self.job_name.clone(),
                    root_cookie: self.ledger.cookie.clone(),
                    sequence: self.ledger.next[1][1],
                    backup_id: backup,
                    request_id: request,
                    generation_binding_sha256: digest,
                }
            };
            self.capture_send(reply)
        }
        fn observe_install(&mut self) -> Result<()> {
            if !matches!(
                self.capture_receive()?,
                RootControlData::BrokerInstalled { .. }
            ) {
                return Err(invalid());
            }
            let _scope = ObservationScope::enter();
            self.critical()?;
            self.recheck_generation(true, false)?;
            let notice = self.ledger.installed.as_ref().ok_or_else(invalid)?;
            self.genesis = Some(HeldGenesisData::read(
                &self.roots.as_ref().ok_or_else(invalid)?.companion,
                notice,
                &self.scan,
            )?);
            self.check_descriptor(self.genesis.as_ref().ok_or_else(invalid)?)?;
            let after = self.observe_host()?;
            self.same_host(&after)?;
            self.critical()?;
            self.recheck_generation(true, false)?;
            let genesis = self.genesis.as_ref().ok_or_else(invalid)?;
            genesis.recheck(&self.scan)?;
            self.check_descriptor(genesis)?;
            let notice = self.ledger.installed.as_ref().ok_or_else(invalid)?;
            let generation = self.install.as_ref().ok_or_else(invalid)?;
            pay(&self.scan, 8192, 24)?;
            let data = InstallCutoverObservationData {
                schema_version: 1,
                capability: "b1_install_cutover_observation_v1".into(),
                job_id: notice.job_id,
                backup_id: notice.backup_id,
                phase: "Install".into(),
                request_id: notice.request_id,
                binding_id: notice.binding_id,
                epoch_id: notice.epoch_id,
                generation_binding_sha256: self.publication_digest(generation.binding)?,
                descriptor_sha256: genesis.descriptor_sha256.clone(),
                epoch_header_sha256: genesis.epoch_header_sha256.clone(),
                genesis_frame_sha256: genesis.genesis_frame_sha256.clone(),
                observed_unix_ms: now_ms(&self.scan)?,
                host_cutover: after.data,
            };
            data.matches_notice(notice)?;
            let dir = generation.directory;
            // Publication enters this same long-lived book before create or
            // sync. Error/unwind therefore cannot drop a partial C11 writer.
            let raw = encode_observation(&data, &self.scan)?;
            let checked = InstallCutoverObservationData::read_data(&raw, &self.scan)?;
            checked.matches_notice(notice)?;
            let slot = self.publish_raw(dir, OBSERVATION_NAME, raw, RECORD_CAP)?;
            self.recheck_generation(true, true)?;
            self.recheck_publication(slot)?;
            self.genesis
                .as_ref()
                .ok_or_else(invalid)?
                .recheck(&self.scan)?;
            let digest = self.publication_digest(slot)?;
            let notice = self.ledger.installed.as_ref().ok_or_else(invalid)?;
            let reply = RootControlData::BrokerInstallObserved {
                schema_version: 1,
                job_id: self.job_name.clone(),
                root_cookie: self.ledger.cookie.clone(),
                sequence: self.ledger.next[1][1],
                backup_id: notice.backup_id.to_string(),
                request_id: notice.request_id.to_string(),
                binding_id: notice.binding_id.to_string(),
                epoch_id: notice.epoch_id.to_string(),
                descriptor_sha256: notice.descriptor_sha256.clone(),
                observation_sha256: digest,
            };
            self.send_original(RootChannel::Capture, &reply)?;
            // Genesis-only reader duty ends at this actual ACK. Root keeps
            // companion/native, both generation locks and the original durable
            // observation record while real broker work may now append frames.
            self.genesis = None;
            self.release_generation_reader(true)?;
            Ok(())
        }
        fn recheck_generation(&self, install: bool, observed: bool) -> Result<()> {
            self.boundary()?;
            let g = if install {
                self.install.as_ref()
            } else {
                self.capture_generation.as_ref()
            }
            .ok_or_else(invalid)?;
            let backup = self.ledger.backup.as_ref().ok_or_else(invalid)?;
            let directory = self.bound_directory(g.directory)?;
            let lock = g.lock.as_ref().ok_or_else(invalid)?;
            let lock_stamp = g.lock_stamp.as_ref().ok_or_else(invalid)?;
            pay(&self.scan, 1024, 10)?;
            if !g.exclusive
                || super::Stamp::read(&lock.metadata()?) != *lock_stamp
                || super::Stamp::read(
                    &directory
                        .open_file(&format!("isolation-{backup}.lock"))?
                        .metadata()?,
                ) != *lock_stamp
            {
                return Err(invalid());
            }
            for slot in [g.proof, g.inspection, g.binding] {
                self.recheck_publication(slot)?;
            }
            if install {
                install_roster(&directory, backup, observed, &self.scan)?;
            } else {
                let runtime = self.runtime_probe.as_ref().and_then(|p| p.record);
                let mut expected = vec![
                    "generation-binding.json".into(),
                    format!("inspection-{backup}.json"),
                    format!("isolation-{backup}.json"),
                    format!("isolation-{backup}.lock"),
                ];
                if let Some(slot) = runtime {
                    self.recheck_publication(slot)?;
                    expected.push(format!("runtime-connect-denied-{backup}.json"));
                }
                expected.sort();
                if roster(&directory, expected.len(), &self.scan)? != expected {
                    return Err(invalid());
                }
            }
            self.boundary()
        }
        fn check_descriptor(&self, g: &HeldGenesisData) -> Result<()> {
            let d = &g.descriptor;
            let capture = self
                .capture
                .as_ref()
                .and_then(|c| c.process.as_ref())
                .ok_or_else(invalid)?;
            let installed = &self.catalog.data.installed_source;
            let roots = self.roots.as_ref().ok_or_else(invalid)?;
            if d.deployment_id.to_string() != installed.deployment_id
                || d.source_group_id.to_string() != installed.source_group_id
                || d.source_application_commit != installed.application_commit
                || d.source_build_sha256 != installed.application_build_sha256
                || d.source_database != installed.database
                || u64::try_from(d.source_database_oid).map_err(|_| invalid())?
                    != installed.database_oid
                || d.source_system_identifier != installed.system_identifier
                || d.broker_pid != capture.namespace_pid
                || d.broker_uid != 0
                || d.broker_gid != 0
                || d.broker_binary_sha256 != hex::encode(capture.codes[0].digest)
                || d.broker_binary_dev != capture.codes[0].stamp.dev
                || d.broker_binary_ino != capture.codes[0].stamp.ino
                || d.broker_binary_bytes != capture.codes[0].stamp.bytes
                || (d.export_root_dev, d.export_root_ino) != roots.export.identity()?
                || (d.companion_root_dev, d.companion_root_ino) != roots.companion.identity()?
                || d.writer_contract_sha256
                    != learning_db::snapshot_broker::RootSnapshotBroker::writer_contract_sha256()
            {
                return Err(invalid());
            }
            // Descriptor effect-lock and listener objects are independently
            // observed from this exact original process/root, not its notice.
            let lock = roots.export.open_file("effect.lock")?;
            pay(&self.scan, 256, 4)?;
            let m = lock.metadata()?;
            if m.uid() != 0
                || m.gid() != 0
                || m.mode() & 0o7777 != 0o600
                || m.nlink() != 1
                || m.len() != 0
                || (m.dev(), m.ino()) != (d.effect_lock_dev, d.effect_lock_ino)
            {
                return Err(invalid());
            }
            drop(lock);
            let marker =
                HeldBytesData::open(&roots.export, "export-root-v2.json", 4096, &self.scan)?;
            let marker_value = api_json(&marker.raw, 4096, &self.scan)?;
            if marker_value["binding_id"].as_str() != Some(d.binding_id.to_string().as_str())
                || marker_value["deployment_id"].as_str()
                    != Some(d.deployment_id.to_string().as_str())
                || marker_value["writer_contract_sha256"].as_str()
                    != Some(d.writer_contract_sha256.as_str())
                || marker_value["export_root_dev"].as_u64() != Some(d.export_root_dev)
                || marker_value["export_root_ino"].as_u64() != Some(d.export_root_ino)
                || marker_value["journal_root_dev"].as_u64() != Some(d.journal_root_dev)
                || marker_value["journal_root_ino"].as_u64() != Some(d.journal_root_ino)
                || marker_value["effect_lock_dev"].as_u64() != Some(d.effect_lock_dev)
                || marker_value["effect_lock_ino"].as_u64() != Some(d.effect_lock_ino)
            {
                return Err(invalid());
            }
            if roster(&roots.export, 2, &self.scan)?
                != ["effect.lock".to_owned(), "export-root-v2.json".to_owned()]
                || d.initial_namespace_sha256
                    != hash(
                        &encode(
                            &vec![
                                ("effect.lock", false, 0_u64),
                                ("export-root-v2.json", false, marker.raw.len() as u64),
                            ],
                            4096,
                            &self.scan,
                        )?,
                        &self.scan,
                    )?
            {
                return Err(invalid());
            }
            drop(marker);
            let sockets = mapped(
                &self.before.as_ref().ok_or_else(invalid)?.current_mounts,
                "/run/kwc4/sfx",
            )?;
            let socketroot = private_dir(&sockets, &self.scan)?;
            let binding = socketroot.open_dir(&d.binding_id.to_string())?;
            let epoch = binding.open_dir(&d.epoch_id.to_string())?;
            if roster(&epoch, 2, &self.scan)?
                != ["management.sock".to_owned(), "work.sock".to_owned()]
            {
                return Err(invalid());
            }
            drop(epoch);
            drop(binding);
            drop(socketroot);
            self.descriptor_socket(
                capture,
                d.work_socket_dev,
                d.work_socket_ino,
                &format!("/run/kwc4/sfx/{}/{}/work.sock", d.binding_id, d.epoch_id),
            )?;
            self.descriptor_socket(
                capture,
                d.management_socket_dev,
                d.management_socket_ino,
                &format!(
                    "/run/kwc4/sfx/{}/{}/management.sock",
                    d.binding_id, d.epoch_id
                ),
            )?;
            if d.source_postmaster_start.is_empty()
                || d.source_postmaster_start.len() > 128
                || !crate::valid_digest(&d.initial_namespace_sha256)
                || !crate::valid_digest(&d.writer_contract_sha256)
            {
                return Err(invalid());
            }
            self.pg
                .as_ref()
                .ok_or_else(invalid)?
                .check_postmaster_start(&d.source_postmaster_start, &self.scan)?;
            Ok(())
        }
        fn descriptor_socket(
            &self,
            capture: &MeasuredProcess,
            dev: u64,
            ino: u64,
            path: &str,
        ) -> Result<()> {
            let raw = self.native.proc.read_relative(
                &format!("{}/net/unix", capture.pid),
                RECORD_CAP,
                &self.scan,
            )?;
            pay(&self.scan, raw.len() * 8, 0)?;
            let text = std::str::from_utf8(&raw).map_err(|_| invalid())?;
            let mut matches = 0;
            for line in text.lines().skip(1) {
                pay(&self.scan, 0, 8)?;
                let mut words = line.split_ascii_whitespace();
                let mut fields = [""; 8];
                let mut count = 0;
                for slot in &mut fields {
                    let Some(word) = words.next() else {
                        break;
                    };
                    *slot = word;
                    count += 1;
                }
                if words.next().is_some() {
                    return Err(invalid());
                }
                if count == 8 && fields[7] == path && fields[6].parse::<u64>().ok() == Some(ino) {
                    if fields[3] != "00010000" || fields[4] != "0001" {
                        return Err(invalid());
                    }
                    matches += 1;
                }
            }
            // sockfs device is a live kernel namespace object, not the named
            // filesystem socket inode. Join actual /proc/fd magic targets.
            if matches != 1 {
                return Err(invalid());
            }
            let dir = self.native.proc.open_relative(
                &format!("{}/fd", capture.pid),
                libc::O_RDONLY | libc::O_DIRECTORY,
                &self.scan,
            )?;
            let mut bytes = [0_u8; 4096];
            let mut found = false;
            loop {
                pay(&self.scan, bytes.len(), 1)?;
                let n = unsafe {
                    libc::syscall(
                        libc::SYS_getdents64,
                        dir.as_raw_fd(),
                        bytes.as_mut_ptr(),
                        bytes.len(),
                    )
                };
                if n < 0 {
                    return Err(invalid());
                }
                if n == 0 {
                    break;
                }
                let mut at = 0;
                while at < n as usize {
                    if at + 19 > n as usize {
                        return Err(invalid());
                    }
                    let len = u16::from_ne_bytes([bytes[at + 16], bytes[at + 17]]) as usize;
                    if len < 20 || at + len > n as usize {
                        return Err(invalid());
                    }
                    let name = &bytes[at + 19..at + len];
                    let end = name.iter().position(|b| *b == 0).ok_or_else(invalid)?;
                    let name = &name[..end];
                    pay(&self.scan, name.len() * 8 + 256, 1)?;
                    if !name.is_empty() && name.iter().all(u8::is_ascii_digit) {
                        let name = CString::new(name).map_err(|_| invalid())?;
                        let raw = unsafe {
                            libc::openat(
                                dir.as_raw_fd(),
                                name.as_ptr(),
                                libc::O_PATH | libc::O_CLOEXEC,
                            )
                        };
                        if raw < 0 {
                            return Err(invalid());
                        }
                        let file = unsafe { File::from_raw_fd(raw) };
                        let m = Stamp::observe(&file, &self.scan)?;
                        if m.dev == dev && m.ino == ino && m.mode & libc::S_IFMT == libc::S_IFSOCK {
                            if found {
                                return Err(invalid());
                            }
                            found = true;
                        }
                    }
                    at += len;
                }
            }
            if !found {
                return Err(invalid());
            }
            Ok(())
        }
        fn publish_runtime_denial(&mut self) -> Result<()> {
            self.critical()?;
            self.recheck_generation(false, false)?;
            let backup = self.ledger.backup.as_ref().ok_or_else(invalid)?.clone();
            let id = Uuid::parse_str(&backup).map_err(|_| invalid())?;
            let generation = self.capture_generation.as_ref().ok_or_else(invalid)?;
            let deadline = generation.deadline.min(self.total);
            let parent = generation.directory;
            self.runtime_probe = Some(RuntimeProbeOwner {
                control: None,
                closed: None,
                socket: None,
                shutdown: None,
                negative_eof: false,
                negative_drained: 0,
                denied: false,
                record: None,
            });
            // The old pipeline publishes Closed only after its real original
            // REVOKE completed. It is a wake-up DATA join, never ACL authority.
            loop {
                root_io_boundary(deadline, &STOP)?;
                pay(&self.scan, 4096, 8)?;
                let control_root = &self.roots.as_ref().ok_or_else(invalid)?.handles["control"];
                match control_root.open_dir(&format!("{backup}.control")) {
                    Ok(directory) => match directory.kind("closed.json") {
                        Ok(learning_assets::backup_fs::BackupEntryKind::File) => {
                            let closed =
                                HeldBytesData::open(&directory, "closed.json", 4096, &self.scan)?;
                            validate_closed_record(&closed.raw, id, &self.scan)?;
                            let identity = directory.identity()?;
                            let owner = self.runtime_probe.as_mut().ok_or_else(invalid)?;
                            owner.control = Some(identity);
                            owner.closed = Some(DataRecord::retain_data(closed));
                            break;
                        }
                        Ok(_) => return Err(invalid()),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(_) => return Err(invalid()),
                    },
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return Err(invalid()),
                }
                self.cooperative_tick(deadline)?;
            }
            self.critical()?;
            self.recheck_generation(false, false)?;
            self.open_original_probe(deadline)?;
            let socket = Arc::clone(
                self.runtime_probe
                    .as_ref()
                    .and_then(|p| p.socket.as_ref())
                    .ok_or_else(invalid)?,
            );
            let pg = self.pg.as_ref().ok_or_else(invalid)?;
            pay(&self.scan, 0, 3)?;
            let mut cred = libc::ucred {
                pid: 0,
                uid: u32::MAX,
                gid: u32::MAX,
            };
            let mut len = std::mem::size_of_val(&cred) as libc::socklen_t;
            if unsafe {
                libc::getsockopt(
                    socket.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_PEERCRED,
                    (&mut cred as *mut libc::ucred).cast(),
                    &mut len,
                )
            } != 0
                || cred.pid != pg.pid as i32
                || cred.uid != 999
                || cred.gid != 999
                || len as usize != std::mem::size_of_val(&cred)
            {
                return Err(invalid());
            }
            pg.recheck(&self.native.proc, &self.scan)?;
            pay(
                &self.scan,
                self.catalog.data.installed_source.database.len() + 256,
                4,
            )?;
            let db = self.catalog.data.installed_source.database.clone();
            let mut startup = Vec::with_capacity(db.len() + 128);
            startup.extend_from_slice(&0_u32.to_be_bytes());
            startup.extend_from_slice(&196608_u32.to_be_bytes());
            startup.extend_from_slice(b"user\0learning_runtime\0database\0");
            startup.extend_from_slice(db.as_bytes());
            startup.extend_from_slice(b"\0\0");
            let size = u32::try_from(startup.len()).map_err(|_| invalid())?;
            startup[..4].copy_from_slice(&size.to_be_bytes());
            self.probe_write(&startup, deadline)?;
            let mut authenticated = false;
            let mut denied = false;
            let mut bytes = 0_usize;
            for _ in 0..8 {
                let mut header = [0_u8; 5];
                self.probe_exact(&mut header, deadline)?;
                let n = u32::from_be_bytes(header[1..].try_into().map_err(|_| invalid())?) as usize;
                if !(4..=4096).contains(&n) {
                    return Err(invalid());
                }
                bytes = bytes
                    .checked_add(n + 1)
                    .filter(|n| *n <= 16384)
                    .ok_or_else(invalid)?;
                pay(&self.scan, n - 4, 1)?;
                let mut body = vec![0_u8; n - 4];
                self.probe_exact(&mut body, deadline)?;
                match header[0] {
                    b'R' if !authenticated && body == 0_u32.to_be_bytes() => authenticated = true,
                    b'E' if authenticated => {
                        connect_acl_denial(&body, &db, &self.scan)?;
                        denied = true;
                        break;
                    }
                    _ => return Err(invalid()),
                }
            }
            if !denied {
                return Err(invalid());
            }
            self.probe_eof(deadline)?;
            self.pg
                .as_ref()
                .ok_or_else(invalid)?
                .recheck(&self.native.proc, &self.scan)?;
            {
                pay(&self.scan, 512, 4)?;
                let directory = self.roots.as_ref().ok_or_else(invalid)?.handles["control"]
                    .open_dir(&format!("{backup}.control"))?;
                let owner = self.runtime_probe.as_mut().ok_or_else(invalid)?;
                if Some(directory.identity()?) != owner.control {
                    return Err(invalid());
                }
                owner.closed.as_ref().ok_or_else(invalid)?.recheck(
                    &directory,
                    "closed.json",
                    &self.scan,
                )?;
                owner.denied = true;
            }
            self.runtime_probe.as_mut().ok_or_else(invalid)?.socket = None;
            self.runtime_probe.as_mut().ok_or_else(invalid)?.shutdown = None;
            drop(socket);
            #[derive(Serialize)]
            struct Probe<'a> {
                backup_id: &'a str,
                database: &'a str,
                result: &'static str,
            }
            let record = encode(
                &Probe {
                    backup_id: &backup,
                    database: &self.catalog.data.installed_source.database,
                    result: "runtime_connect_denied",
                },
                1024,
                &self.scan,
            )?;
            root_io_boundary(deadline, &STOP)?;
            let slot = self.publish_raw(
                parent,
                &format!("runtime-connect-denied-{backup}.json"),
                record,
                1024,
            )?;
            self.recheck_publication(slot)?;
            root_io_boundary(deadline, &STOP)?;
            let owner = self.runtime_probe.as_mut().ok_or_else(invalid)?;
            owner.record = Some(slot);
            // Only after real authenticated ACL denial + physical EOF + durable
            // record have this fixed probe's socket/reader duties completed.
            owner.socket = None;
            owner.closed = None;
            owner.control = None;
            Ok(())
        }
        fn observe_captured_data_from_original_artifacts(
            &mut self,
        ) -> Result<RootCapturedDataSummary> {
            // No parameters: the notification digest is already held by this
            // exact ledger. Every field below comes from our original fixed
            // root mappings, job/backup, source metadata, pin and SFX bytes.
            self.critical()?;
            let backup = self.ledger.backup.as_ref().ok_or_else(invalid)?.clone();
            let id = Uuid::parse_str(&backup).map_err(|_| invalid())?;
            let after = self.observe_host()?;
            self.same_host(&after)?;
            let roots = self.roots.as_ref().ok_or_else(invalid)?;
            pay(&self.scan, 8192, 32)?;
            let control = &roots.handles["control"];
            let evidence = control.open_dir(&format!("{backup}.b1-captured"))?;
            let source_names = [
                "protection-catalog.json",
                "protection-pending.json",
                "protection-retained.json",
                "protection-snapshot.json",
                "source-authority.json",
                "source-generation.json",
                "source-group.json",
                "source-released.json",
            ];
            let mut records = Vec::new();
            let mut source_raw = BTreeMap::new();
            for name in source_names {
                let raw = artifact_bytes(
                    &evidence,
                    name,
                    RECORD_CAP,
                    self.operation_deadline(),
                    &self.scan,
                )?;
                records.push(crate::FileRecord {
                    path: name.into(),
                    size: raw.len() as u64,
                    sha256: hash(&raw, &self.scan)?,
                });
                source_raw.insert(name.to_owned(), raw);
            }
            let group: registry::SourceGroup =
                parse(&source_raw["source-group.json"], 4096, &self.scan)?;
            if group != roots.group_data
                || hash(&source_raw["source-group.json"], &self.scan)?
                    != self
                        .catalog
                        .data
                        .installed_source
                        .source_group_record_sha256
                || hash(&source_raw["source-authority.json"], &self.scan)?
                    != self.catalog.data.installed_source.registry_authority_sha256
            {
                return Err(invalid());
            }
            let generation: registry::Generation =
                parse(&source_raw["source-generation.json"], 524288, &self.scan)?;
            if generation.format_version != 1
                || generation.capability != "backup_registry_generation_v1"
                || generation.deployment_id != group.deployment_id
                || !(1..=128).contains(&generation.generation)
                || !generation.groups.iter().any(|x| {
                    x.id == group.group_id
                        && x.sha256
                            == self
                                .catalog
                                .data
                                .installed_source
                                .source_group_record_sha256
                })
            {
                return Err(invalid());
            }
            let original_generations = roots.registry.open_dir("generations")?;
            let original_generation = HeldBytesData::open(
                &original_generations,
                &format!("{:020}.json", generation.generation),
                524288,
                &self.scan,
            )?;
            if original_generation.raw != source_raw["source-generation.json"] {
                return Err(invalid());
            }
            drop(original_generation);
            drop(original_generations);
            for (kind, root) in &roots.records {
                let name = format!("roots/{}.json", root.enrollment_id);
                let raw = artifact_bytes(
                    &evidence,
                    &name,
                    4096,
                    self.operation_deadline(),
                    &self.scan,
                )?;
                let actual: registry::RootRecord = parse(&raw, 4096, &self.scan)?;
                if actual != *root || group.roots.get(kind) != Some(&root.enrollment_id) {
                    return Err(invalid());
                }
                let digest = hash(&raw, &self.scan)?;
                if !generation
                    .roots
                    .iter()
                    .any(|x| x.id == root.enrollment_id && x.sha256 == digest)
                {
                    return Err(invalid());
                }
                records.push(crate::FileRecord {
                    path: name.clone(),
                    size: raw.len() as u64,
                    sha256: digest,
                });
                source_raw.insert(name, raw);
            }
            records.sort_by(|a, b| a.path.cmp(&b.path));
            if records.len() != 12 {
                return Err(invalid());
            }
            exact_artifact_tree(
                &evidence,
                "",
                &records,
                0,
                self.operation_deadline(),
                &self.scan,
            )?;
            drop(evidence);
            let pin = roots.handles["local_pins"].open_dir(&format!("{backup}.sealed"))?;
            let manifest_raw = artifact_bytes(
                &pin,
                "manifest.json",
                64 * 1024 * 1024,
                self.operation_deadline(),
                &self.scan,
            )?;
            pay(&self.scan, manifest_raw.len() * 64 + 4096, 0)?;
            let nodes = manifest_raw
                .iter()
                .filter(|b| matches!(**b, b'{' | b'[' | b',' | b':'))
                .count()
                + 1;
            pay(&self.scan, 0, nodes)?;
            let manifest: crate::BackupManifestV1 = serde_json::from_slice(&manifest_raw)?;
            if manifest.backup_id != id
                || manifest.capability != crate::BACKUP_CAPABILITY
                || manifest.format_version != 1
                || manifest.source.application_commit
                    != self.catalog.data.installed_source.application_commit
                || manifest.source.application_build_sha256
                    != self.catalog.data.installed_source.application_build_sha256
                || manifest.files.len() >= 100_000
            {
                return Err(invalid());
            }
            pay(&self.scan, manifest_raw.len() * 2, 1)?;
            if manifest.canonical_bytes()? != manifest_raw {
                return Err(invalid());
            }
            let index = artifact_bytes(
                &pin,
                "asset-index.json",
                64 * 1024 * 1024,
                self.operation_deadline(),
                &self.scan,
            )?;
            pay(
                &self.scan,
                index.len() * 64 + 4096,
                manifest.files.len() + 32,
            )?;
            manifest.validate_with_index(&index)?;
            let manifest_sha256 = hash(&manifest_raw, &self.scan)?;
            pay(&self.scan, manifest.files.len() * 256, manifest.files.len())?;
            let mut pin_records = manifest.files.clone();
            pin_records.push(crate::FileRecord {
                path: "manifest.json".into(),
                size: manifest_raw.len() as u64,
                sha256: manifest_sha256.clone(),
            });
            pin_records.sort_by(|a, b| a.path.cmp(&b.path));
            exact_artifact_tree(
                &pin,
                "",
                &pin_records,
                0,
                self.operation_deadline(),
                &self.scan,
            )?;
            let pin_identity = pin.identity()?;
            validate_m5_source_data(
                &source_raw,
                &group,
                &generation,
                &manifest,
                &index,
                pin_identity,
                &self.scan,
            )?;
            drop(pin);
            let captures = roots.companion.open_dir("captures")?;
            if roster(&captures, 1, &self.scan)? != [backup.clone()] {
                return Err(invalid());
            }
            let effects = captures.open_dir(&backup)?;
            drop(captures);
            let capture_raw = artifact_bytes(
                &effects,
                "capture.json",
                64 * 1024 * 1024,
                self.operation_deadline(),
                &self.scan,
            )?;
            let capture: crate::source::snapshot_effects::wire::Capture =
                parse(&capture_raw, 64 * 1024 * 1024, &self.scan)?;
            if capture.format_version != 2
                || capture.capability != "snapshot_effect_capture_v2"
                || capture.storage_encoding != "root_frame_v1"
                || capture.backup_id != id
                || capture.manifest_sha256 != manifest_sha256
                || capture.source_group_id.to_string() != group.group_id
                || capture.source_registry_generation_sha256
                    != hash(&source_raw["source-generation.json"], &self.scan)?
                || capture.files.len() >= 100_000
            {
                return Err(invalid());
            }
            pay(&self.scan, capture.files.len() * 256, capture.files.len())?;
            let mut effect_records = capture.files.clone();
            effect_records.push(crate::FileRecord {
                path: "capture.json".into(),
                size: capture_raw.len() as u64,
                sha256: hash(&capture_raw, &self.scan)?,
            });
            effect_records.sort_by(|a, b| a.path.cmp(&b.path));
            exact_artifact_tree(
                &effects,
                "",
                &effect_records,
                0,
                self.operation_deadline(),
                &self.scan,
            )?;
            // Re-read the complete binary prefix from this same sealed bundle.
            // Prefix::check retains its original sequence>=2 rule; it is never
            // used by the genesis-only install reader.
            let mut prefix_files = BTreeMap::new();
            for record in &capture.files {
                if record.path.starts_with("journal/") {
                    let raw = artifact_bytes(
                        &effects,
                        &record.path,
                        FRAME_CAP,
                        self.operation_deadline(),
                        &self.scan,
                    )?;
                    prefix_files.insert(record.path.clone(), raw);
                }
            }
            let descriptor_raw = artifact_bytes(
                &effects,
                "epoch.json",
                FRAME_CAP,
                self.operation_deadline(),
                &self.scan,
            )?;
            let ledger_raw = artifact_bytes(
                &effects,
                "ledger.json",
                64 * 1024 * 1024,
                self.operation_deadline(),
                &self.scan,
            )?;
            let ledger: crate::source::snapshot_effects::wire::Ledger =
                parse(&ledger_raw, 64 * 1024 * 1024, &self.scan)?;
            let head = crate::source::snapshot_effects::wire::Reference {
                sequence: capture.journal_head_sequence,
                sha256: capture.journal_head_sha256.clone(),
            };
            prepay_original_prefix(&prefix_files, &descriptor_raw, &self.scan)?;
            crate::source::snapshot_effects::wire::Prefix::check(
                &prefix_files,
                &descriptor_raw,
                ledger.epoch_id,
                ledger.binding_id,
                &head,
            )?;
            let original = self.ledger.installed.as_ref().ok_or_else(invalid)?;
            if ledger.epoch_id != original.epoch_id
                || ledger.binding_id != original.binding_id
                || hash(&descriptor_raw, &self.scan)? != original.descriptor_sha256
            {
                return Err(invalid());
            }
            drop(effects);
            let capture_generation = self.capture_generation.as_ref().ok_or_else(invalid)?;
            let data = job::CapturedData {
                schema_version: 1,
                capability: "b1_captured_input_data_only_v1".into(),
                job_id: self.job_name.clone(),
                backup_id: backup,
                capture_start_sha256: self
                    .publication_digest(self.capture_publication.ok_or_else(invalid)?)?,
                capture_generation_sha256: self.publication_digest(capture_generation.binding)?,
                source_group_id: group.group_id,
                source_registry_generation_sha256: hash(
                    &source_raw["source-generation.json"],
                    &self.scan,
                )?,
                manifest_sha256,
                captured_roster_sha256: hash(
                    &encode(&records, RECORD_CAP, &self.scan)?,
                    &self.scan,
                )?,
                snapshot_effects_roster_sha256: hash(
                    &encode(&effect_records, RECORD_CAP, &self.scan)?,
                    &self.scan,
                )?,
            };
            data.validate()?;
            let raw = encode(&data, 4096, &self.scan)?;
            let digest = digest_bytes(&raw, &self.scan)?;
            if self.ledger.captured.as_deref() != Some(hex::encode(digest).as_str()) {
                return Err(invalid());
            }
            self.release_generation_reader(false)?;
            self.critical()?;
            Ok(RootCapturedDataSummary { data, raw, digest })
        }
        fn settle_capture(&mut self) -> Result<usize> {
            self.critical()?;
            if !matches!(
                self.source_receive()?,
                RootControlData::SettleCapture { .. }
            ) {
                return Err(invalid());
            }
            let backup = self.ledger.backup.as_ref().ok_or_else(invalid)?.clone();
            let captured = self.captured.as_ref().ok_or_else(invalid)?;
            let digest = hex::encode(captured.digest);
            let fixed = self.operation_deadline();
            self.settlement_deadline = Some(self.settlement_deadline.unwrap_or(fixed).min(fixed));
            self.capture_send(RootControlData::SettleCapture {
                schema_version: 1,
                job_id: self.job_name.clone(),
                root_cookie: self.ledger.cookie.clone(),
                sequence: self.ledger.next[1][1],
                backup_id: backup.clone(),
                captured_data_sha256: digest.clone(),
            })?;
            let deadline = self.operation_deadline().min(self.total);
            let control = self.control_fd(RootChannel::Capture)?;
            self.capture
                .as_ref()
                .and_then(|c| c.attached.as_ref())
                .ok_or_else(invalid)?
                .input_shutdown
                .close_once()?;
            // A real EOF must arrive on the logical stdout bridge. The mux
            // only closes it after frame-complete physical attached EOF and
            // bounded stderr drain. Any queued byte is a permanent negative.
            loop {
                root_io_boundary(deadline, &STOP)?;
                self.supervise_settlement_waiting()?;
                self.ledger.wire.extra_probe(&self.scan)?;
                let mut extra = 0_u8;
                let n = unsafe {
                    libc::recv(
                        control,
                        (&mut extra as *mut u8).cast(),
                        1,
                        libc::MSG_DONTWAIT,
                    )
                };
                if n == 0 {
                    break;
                }
                if n > 0 {
                    return Err(invalid());
                }
                match std::io::Error::last_os_error().kind() {
                    std::io::ErrorKind::Interrupted => continue,
                    std::io::ErrorKind::WouldBlock => {
                        pay(&self.scan, 0, 1)?;
                        let mut p = libc::pollfd {
                            fd: control,
                            events: libc::POLLIN,
                            revents: 0,
                        };
                        let n = unsafe { libc::poll(&mut p, 1, 100) };
                        if n < 0
                            && std::io::Error::last_os_error().kind()
                                != std::io::ErrorKind::Interrupted
                        {
                            return Err(invalid());
                        }
                    }
                    _ => return Err(invalid()),
                }
            }
            loop {
                root_io_boundary(deadline, &STOP)?;
                self.supervise_settlement_waiting()?;
                let (running, exit, pid) = self.original_inspect_exec()?;
                let owner = self.capture.as_mut().ok_or_else(invalid)?;
                let process = owner.process.as_ref().ok_or_else(invalid)?;
                pay(&self.scan, 0, 1)?;
                let mut p = libc::pollfd {
                    fd: process.pidfd.as_ref().ok_or_else(invalid)?.fd.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                let status = unsafe { libc::poll(&mut p, 1, 0) };
                if status < 0 || p.revents & (libc::POLLNVAL | libc::POLLERR) != 0 {
                    return Err(invalid());
                }
                if !running {
                    if exit != Some(0)
                        || (pid != 0 && pid != process.pid)
                        || status != 1
                        || p.revents & libc::POLLIN == 0
                    {
                        return Err(invalid());
                    }
                    if owner.attached.as_mut().ok_or_else(invalid)?.joined()? {
                        owner.natural = true;
                        break;
                    }
                }
                pay(&self.scan, 0, 1)?;
                let mut poll = libc::pollfd {
                    fd: process.pidfd.as_ref().ok_or_else(invalid)?.fd.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                unsafe { libc::poll(&mut poll, 1, 10) };
            }
            let owner = self.capture.as_ref().ok_or_else(invalid)?;
            if !owner.natural {
                return Err(invalid());
            }
            let result = owner
                .attached
                .as_ref()
                .and_then(|a| a.result.as_ref())
                .ok_or_else(invalid)?;
            let record = job::CaptureSettlement {
                schema_version: 1,
                capability: "b1_capture_natural_settlement_observation_v1".into(),
                job_id: self.job_name.clone(),
                backup_id: backup,
                capture_start_sha256: self
                    .publication_digest(self.capture_publication.ok_or_else(invalid)?)?,
                captured_data_sha256: digest,
                exec_id: owner.exec_id.clone(),
                child_start_ticks: owner.process.as_ref().ok_or_else(invalid)?.start,
                natural_exit_code: 0,
                stdout_eof: result.double_eof,
                stderr_eof: result.double_eof,
                extra_bytes: !result.no_extra,
                owner_settled: true,
                stdout_sha256: hex::encode(result.stdout),
                stderr_sha256: hex::encode(result.stderr),
            };
            record.validate()?;
            let slot = self.publish(
                self.backup_slot.ok_or_else(invalid)?,
                "capture-settlement.json",
                &record,
                4096,
            )?;
            // These original resources have completed the positive settlement
            // gate above, including durable readback. Releasing them here does
            // not provide any evidence of exit or EOF.
            let owner = self.capture.as_mut().ok_or_else(invalid)?;
            owner.process = None;
            owner.bootstrap_pidfd = None;
            owner.attached = None;
            owner.transport = None;
            owner.physical_shutdown = None;
            Ok(slot)
        }
        fn supervise_settlement_waiting(&mut self) -> Result<()> {
            if !(15..=16).contains(&self.ledger.step) {
                return Err(invalid());
            }
            self.critical()?;
            self.cooperative_tick(self.operation_deadline())?;
            self.boundary()
        }
        fn finish_source(&mut self, settlement: usize) -> Result<()> {
            // Fix this original earlier bound before the notification clears
            // ledger.pending. Every nested operation uses operation_deadline.
            let deadline = self.operation_deadline().min(
                Instant::now()
                    .checked_add(Duration::from_secs(30))
                    .ok_or_else(invalid)?,
            );
            self.phase_deadline = Some(deadline);
            root_io_boundary(deadline, &STOP)?;
            let backup = self.ledger.backup.as_ref().ok_or_else(invalid)?.clone();
            let digest = self.publication_digest(settlement)?;
            self.source_send(RootControlData::CaptureSettled {
                schema_version: 1,
                job_id: self.job_name.clone(),
                root_cookie: self.ledger.cookie.clone(),
                sequence: self.ledger.next[0][1],
                backup_id: backup,
                capture_settlement_sha256: digest,
            })?;
            root_io_boundary(deadline, &STOP)?;
            let control = self.control_fd(RootChannel::Source)?;
            let mut eof = false;
            let mut status = None;
            loop {
                root_io_boundary(deadline, &STOP)?;
                if !eof {
                    self.ledger.wire.extra_probe(&self.scan)?;
                    let mut extra = 0_u8;
                    let n = unsafe {
                        libc::recv(
                            control,
                            (&mut extra as *mut u8).cast(),
                            1,
                            libc::MSG_DONTWAIT,
                        )
                    };
                    if n == 0 {
                        eof = true;
                    } else if n > 0
                        || !matches!(
                            std::io::Error::last_os_error().kind(),
                            std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                        )
                    {
                        return Err(invalid());
                    }
                }
                if status.is_none() {
                    pay(&self.scan, 0, 1)?;
                    status = self.source.as_mut().ok_or_else(invalid)?.child.try_wait()?;
                    if status.is_some() {
                        self.source.as_mut().ok_or_else(invalid)?.waited = true;
                    }
                }
                self.supervise_settlement_waiting()?;
                root_io_boundary(deadline, &STOP)?;
                if eof && status.is_some() {
                    break;
                }
                pay(&self.scan, 0, 1)?;
                let mut p = libc::pollfd {
                    fd: control,
                    events: if eof { 0 } else { libc::POLLIN },
                    revents: 0,
                };
                unsafe { libc::poll(&mut p, 1, 10) };
            }
            root_io_boundary(deadline, &STOP)?;
            if status.and_then(|s| s.code()) != Some(0) {
                return Err(invalid());
            }
            self.check_job_lock()?;
            self.boundary()?;
            root_io_boundary(deadline, &STOP)?;
            Ok(())
        }
        fn execute(&mut self) -> Result<()> {
            self.spawn_source()?;
            self.start_capture()?;
            self.issue_generation(true)?;
            self.observe_install()?;
            self.issue_generation(false)?;
            self.publish_runtime_denial()?;
            self.critical()?;
            if !matches!(
                self.capture_receive()?,
                RootControlData::CapturedDataReady { .. }
            ) {
                return Err(invalid());
            }
            let summary = self.observe_captured_data_from_original_artifacts()?;
            pay(&self.scan, summary.raw.len() + 128, 1)?;
            let raw = summary.raw.clone();
            let digest = hex::encode(summary.digest);
            let backup = summary.data.backup_id.clone();
            self.captured = Some(summary);
            let slot = self.publish_raw(
                self.backup_slot.ok_or_else(invalid)?,
                "captured-data.json",
                raw,
                4096,
            )?;
            if self.publication_digest(slot)? != digest {
                return Err(invalid());
            }
            self.source_send(RootControlData::CapturedDataReady {
                schema_version: 1,
                job_id: self.job_name.clone(),
                root_cookie: self.ledger.cookie.clone(),
                sequence: self.ledger.next[0][1],
                backup_id: backup,
                captured_data_sha256: digest,
            })?;
            let settlement = self.settle_capture()?;
            self.finish_source(settlement)
        }
        fn negative_abort_once(&mut self, channel: RootChannel, deadline: Instant) -> Result<()> {
            let c = usize::from(channel == RootChannel::Capture);
            let closed = match channel {
                RootChannel::Source => self
                    .source_shutdown
                    .as_ref()
                    .map(|s| s.state.load(Ordering::Acquire) == 2),
                RootChannel::Capture => self
                    .capture
                    .as_ref()
                    .and_then(|c| c.attached.as_ref())
                    .map(|a| a.input_shutdown.state.load(Ordering::Acquire) == 2),
            }
            .unwrap_or(false);
            if closed {
                self.abort_attempted[c] = true;
                return Ok(());
            }
            if self.abort_attempted[c] || self.outbound_partial[c] {
                return Ok(());
            }
            self.abort_attempted[c] = true;
            let fd = self.control_fd(channel)?;
            let data = RootControlData::Abort {
                schema_version: 1,
                job_id: self.job_name.clone(),
                root_cookie: self.ledger.cookie.clone(),
                sequence: self.ledger.next[c][1],
                reason_code: "SettlementUnconfirmed".into(),
            };
            let raw = encode(&data, CONTROL_CAP, &self.scan)?;
            self.ledger.wire.prefix(&self.scan)?;
            self.ledger.wire.body(raw.len(), &self.scan)?;
            self.ledger.next[c][1] = self.ledger.next[c][1].checked_add(1).ok_or_else(invalid)?;
            self.outbound_partial[c] = true;
            let prefix = (raw.len() as u32).to_be_bytes();
            for part in [&prefix[..], &raw[..]] {
                let mut bytes = part;
                while !bytes.is_empty() {
                    root_io_boundary(deadline, &STOP)?;
                    pay(&self.scan, bytes.len(), 1)?;
                    let n = unsafe {
                        libc::send(
                            fd,
                            bytes.as_ptr().cast(),
                            bytes.len(),
                            libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
                        )
                    };
                    if n > 0 {
                        bytes = &bytes[n as usize..];
                        continue;
                    }
                    if n == 0 {
                        return Err(invalid());
                    }
                    match std::io::Error::last_os_error().kind() {
                        std::io::ErrorKind::Interrupted => continue,
                        std::io::ErrorKind::WouldBlock => {
                            let mut p = libc::pollfd {
                                fd,
                                events: libc::POLLOUT,
                                revents: 0,
                            };
                            pay(&self.scan, 0, 1)?;
                            unsafe { libc::poll(&mut p, 1, 10) };
                        }
                        _ => return Err(invalid()),
                    }
                }
            }
            self.outbound_partial[c] = false;
            if channel == RootChannel::Capture
                && let Some(a) = self.capture.as_mut().and_then(|c| c.attached.as_mut())
            {
                a.expected_stdin = a
                    .expected_stdin
                    .checked_add(raw.len() + 4)
                    .ok_or_else(invalid)?;
            }
            match channel {
                RootChannel::Source => {
                    if let Some(owner) = &self.source_shutdown {
                        owner.close_once()?;
                    }
                }
                RootChannel::Capture => {
                    if let Some(owner) = self.capture.as_ref().and_then(|c| c.attached.as_ref()) {
                        owner.input_shutdown.close_once()?;
                    }
                }
            }
            Ok(())
        }
        fn negative_drain(&mut self, channel: RootChannel, eof: &mut bool) -> Result<()> {
            if *eof {
                return Ok(());
            }
            let fd = self.control_fd(channel)?;
            for _ in 0..256 {
                self.ledger.wire.extra_probe(&self.scan)?;
                pay(&self.scan, 1, 1)?;
                let mut byte = 0_u8;
                let n =
                    unsafe { libc::recv(fd, (&mut byte as *mut u8).cast(), 1, libc::MSG_DONTWAIT) };
                if n == 0 {
                    *eof = true;
                    break;
                }
                if n > 0 {
                    continue;
                }
                match std::io::Error::last_os_error().kind() {
                    std::io::ErrorKind::Interrupted => continue,
                    std::io::ErrorKind::WouldBlock => break,
                    _ => return Err(invalid()),
                }
            }
            Ok(())
        }

        fn negative_cleanup(&mut self, deadline: Instant) -> Result<bool> {
            // EOF revocation has already consumed only original prepaid slots.
            // Abort, when still possible, has its own original wire/scan bill.
            if self.source.is_some() {
                let _ = self.negative_abort_once(RootChannel::Source, deadline);
            }
            if self
                .capture
                .as_ref()
                .is_some_and(|c| c.attached.is_some() && !c.natural)
            {
                let _ = self.negative_abort_once(RootChannel::Capture, deadline);
            }
            let mut source_eof = self.source.is_none();
            let mut logical_capture_eof = self.capture.is_none();
            let mut physical_capture_eof = self.capture.is_none();
            let mut capture_stopped = self.capture.is_none();
            let mut worker_done = self.capture.is_none();
            loop {
                root_io_boundary(deadline, &STOP)?;
                self.cooperative_tick(deadline)?;
                if self.source.is_some() {
                    self.negative_drain(RootChannel::Source, &mut source_eof)?;
                }
                if self.capture.as_ref().is_some_and(|c| c.attached.is_some()) {
                    self.negative_drain(RootChannel::Capture, &mut logical_capture_eof)?;
                }
                if let Some(source) = self.source.as_mut()
                    && !source.waited
                {
                    pay(&self.scan, 0, 1)?;
                    if source.child.try_wait()?.is_some() {
                        source.waited = true;
                    }
                }
                self.advance_original_tools()?;
                let tools_done = self.tools.iter().all(|tool| {
                    tool.status.is_some()
                        && (tool.stdout.is_none() || tool.out_eof)
                        && (tool.stderr.is_none() || tool.err_eof)
                });
                // A failed foreground message is never replayed. Real raw EOF
                // ends that reader duty before an optional exact-exec GET.
                if self.http.as_ref().is_some_and(|h| h.negative_eof) && !self.http_effect_unknown {
                    self.http = None;
                }
                if self.runtime_probe.as_ref().is_some_and(|p| p.negative_eof)
                    && let Some(probe) = self.runtime_probe.as_mut()
                {
                    probe.socket = None;
                    probe.shutdown = None;
                }
                if self
                    .host_scan
                    .as_ref()
                    .and_then(|h| h.http.as_ref())
                    .is_some_and(|h| h.negative_eof)
                {
                    self.host_scan.as_mut().ok_or_else(invalid)?.http = None;
                }
                let inspect = if self
                    .capture
                    .as_ref()
                    .is_some_and(|c| !c.natural && c.bootstrap.is_some())
                    && self.http.is_none()
                    && self
                        .runtime_probe
                        .as_ref()
                        .is_none_or(|p| p.socket.is_none())
                {
                    Some(self.original_inspect_exec()?)
                } else {
                    None
                };
                if let Some(capture) = self.capture.as_mut() {
                    if capture.natural {
                        capture_stopped = true;
                        logical_capture_eof = true;
                        physical_capture_eof = true;
                        worker_done = true;
                    } else {
                        if let Some(attached) = capture.attached.as_mut() {
                            if attached
                                .worker
                                .as_ref()
                                .is_some_and(JoinHandle::is_finished)
                            {
                                let joined = attached.worker.take().ok_or_else(invalid)?.join();
                                worker_done = true;
                                if let Ok(Ok(result)) = joined {
                                    physical_capture_eof = result.double_eof;
                                    attached.result = Some(result);
                                }
                            }
                            // Logical bridge EOF was a local negative signal.
                            // Only this same physical stream's actual EOF, or
                            // a successfully joined decoder's physical result,
                            // can fill its separate observation balance.
                            if worker_done && !physical_capture_eof {
                                let mut bytes = [0_u8; 256];
                                pay(&self.scan, bytes.len(), 1)?;
                                let n = unsafe {
                                    libc::recv(
                                        attached.transport.as_raw_fd(),
                                        bytes.as_mut_ptr().cast(),
                                        bytes.len(),
                                        libc::MSG_DONTWAIT,
                                    )
                                };
                                if n == 0 {
                                    physical_capture_eof = true;
                                } else if n > 0 {
                                    capture.negative_physical_drained = capture
                                        .negative_physical_drained
                                        .checked_add(n as usize)
                                        .filter(|n| {
                                            *n <= CONTROL_BYTES as usize + 1024 * 1024 + 65536
                                        })
                                        .ok_or_else(invalid)?;
                                } else if !matches!(
                                    std::io::Error::last_os_error().kind(),
                                    std::io::ErrorKind::Interrupted
                                        | std::io::ErrorKind::WouldBlock
                                ) {
                                    return Err(invalid());
                                }
                            }
                        }
                        if let Some((running, exit, pid)) = inspect {
                            let original = capture.bootstrap.ok_or_else(invalid)?;
                            let owner = capture
                                .process
                                .as_ref()
                                .and_then(|p| p.pidfd.as_deref())
                                .or(capture.bootstrap_pidfd.as_deref())
                                .ok_or_else(invalid)?;
                            pay(&self.scan, 0, 1)?;
                            let mut p = libc::pollfd {
                                fd: owner.fd.as_raw_fd(),
                                events: libc::POLLIN,
                                revents: 0,
                            };
                            let n = unsafe { libc::poll(&mut p, 1, 0) };
                            if n < 0 || p.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
                                return Err(invalid());
                            }
                            capture_stopped = !running
                                && exit.is_some()
                                && (pid == 0 || pid == original.0)
                                && n == 1
                                && p.revents & libc::POLLIN != 0;
                        }
                    }
                }
                let readers_done = self.http.as_ref().is_none_or(|h| h.negative_eof)
                    && self
                        .host_scan
                        .as_ref()
                        .and_then(|h| h.http.as_ref())
                        .is_none_or(|h| h.negative_eof)
                    && self
                        .runtime_probe
                        .as_ref()
                        .is_none_or(|p| p.socket.is_none() || p.negative_eof)
                    && self.daemon_bootstrap.is_none();
                root_io_boundary(deadline, &STOP)?;
                if !self.http_effect_unknown
                    && readers_done
                    && source_eof
                    && self.source.as_ref().is_none_or(|s| s.waited)
                    && logical_capture_eof
                    && physical_capture_eof
                    && capture_stopped
                    && worker_done
                    && tools_done
                {
                    return Ok(true);
                }
            }
        }

        fn revoke_original_writes(&self) {
            // Costs and exact socket bindings already belong to this Book.
            // No clocks, parsing, scan retry, normal message or fresh FD here.
            if let Some(capture) = &self.capture {
                if let Some(physical) = &capture.physical_shutdown {
                    let _ = physical.close_once();
                }
                if let Some(attached) = &capture.attached {
                    let _ = attached.input_shutdown.close_once();
                    let _ = attached.output_shutdown.close_once();
                }
            }
            if let Some(source) = &self.source_shutdown {
                let _ = source.close_once();
            }
            // These are separately prepaid reader closures, not core slot4.
            if let Some(http) = &self.http {
                let _ = http.shutdown.close_once();
            }
            if let Some(http) = self.host_scan.as_ref().and_then(|h| h.http.as_ref()) {
                let _ = http.shutdown.close_once();
            }
            if let Some(probe) = self
                .runtime_probe
                .as_ref()
                .and_then(|p| p.shutdown.as_ref())
            {
                let _ = probe.close_once();
            }
            if let Some(bootstrap) = &self.daemon_shutdown {
                let _ = bootstrap.close_once();
            }
        }
        fn advance_negative_readers(&mut self) -> Result<()> {
            let deadline = self.operation_deadline();
            root_io_boundary(deadline, &STOP)?;
            if let Some(owner) = self.http.as_mut() {
                negative_http_drain(owner, deadline, &self.scan)?;
            }
            {
                let _scope = ObservationScope::enter();
                if let Some(owner) = self.host_scan.as_mut().and_then(|h| h.http.as_mut()) {
                    negative_http_drain(owner, deadline, &self.scan)?;
                }
            }
            if let Some(owner) = self.runtime_probe.as_mut()
                && !owner.negative_eof
                && let Some(socket) = &owner.socket
            {
                let mut bytes = [0_u8; 256];
                pay(&self.scan, bytes.len(), 1)?;
                let n = unsafe {
                    libc::recv(
                        socket.as_raw_fd(),
                        bytes.as_mut_ptr().cast(),
                        bytes.len(),
                        libc::MSG_DONTWAIT,
                    )
                };
                if n == 0 {
                    owner.negative_eof = true;
                } else if n > 0 {
                    owner.negative_drained = owner
                        .negative_drained
                        .checked_add(n as usize)
                        .filter(|n| *n <= 16384)
                        .ok_or_else(invalid)?;
                } else if !matches!(
                    std::io::Error::last_os_error().kind(),
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                ) {
                    return Err(invalid());
                }
            }
            if let Some(socket) = &self.daemon_bootstrap {
                let mut bytes = [0_u8; 256];
                pay(&self.scan, bytes.len(), 1)?;
                let n = unsafe {
                    libc::recv(
                        socket.as_raw_fd(),
                        bytes.as_mut_ptr().cast(),
                        bytes.len(),
                        libc::MSG_DONTWAIT,
                    )
                };
                if n == 0 {
                    self.daemon_bootstrap = None;
                    self.daemon_shutdown = None;
                } else if n > 0 {
                    self.daemon_negative_drained = self
                        .daemon_negative_drained
                        .checked_add(n as usize)
                        .filter(|n| *n <= 8192)
                        .ok_or_else(invalid)?;
                } else if !matches!(
                    std::io::Error::last_os_error().kind(),
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                ) {
                    return Err(invalid());
                }
            }
            root_io_boundary(deadline, &STOP)
        }

        fn retain_hold(&mut self) -> Result<()> {
            self.ledger.failed = true;
            self.sticky_negative = true;
            self.failed_io.store(true, Ordering::Release);
            self.hold_attempted = true;
            let first = Instant::now()
                .checked_add(Duration::from_secs(30))
                .unwrap_or(self.total);
            let deadline = self
                .total
                .min(self.negative_deadline.unwrap_or(first))
                .min(self.settlement_deadline.unwrap_or(self.total))
                .min(self.phase_deadline.unwrap_or(self.total));
            self.negative_deadline = Some(deadline);
            self.revoke_original_writes();
            let _cleanup = CleanupScope::enter();
            // This immutable record states the actual unknown balance at the
            // moment normal work becomes sticky-negative. Publish before any
            // bounded close/drain/wait can consume the remaining original time.
            let hold = job::Hold {
                schema_version: 1,
                capability: "b1_source_job_hold_v1".into(),
                job_id: self.job_name.clone(),
                phase: if self.capture.is_some() {
                    "Capture"
                } else {
                    "Bootstrap"
                }
                .into(),
                reason_code: "SettlementUnconfirmed".into(),
                retry_allowed: false,
                owner_settlement: "Unconfirmed".into(),
            };
            self.hold_durable = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                hold.validate()?;
                self.publish(self.job_slot, "hold.json", &hold, 4096)
                    .map(|_| ())
            }))
            .ok()
            .is_some_and(|result| result.is_ok());
            let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.negative_cleanup(deadline)
            }))
            .ok()
            .and_then(std::result::Result::ok)
            .unwrap_or(false);
            if stopped {
                return Err(invalid());
            }
            // Unknown owners stay in this exact stack. The elapsed cleanup
            // window, failed Drop or a signal cannot become successful stop.
            loop {
                std::thread::park_timeout(Duration::from_secs(30));
            }
        }
    }
    fn idle_socket(
        socket: &UnixStream,
        wire: &mut RootWireBudget,
        scan: &SharedJournalBudget,
    ) -> Result<()> {
        wire.extra_probe(scan)?;
        let mut byte = 0_u8;
        let n = unsafe {
            libc::recv(
                socket.as_raw_fd(),
                (&mut byte as *mut u8).cast(),
                1,
                libc::MSG_PEEK | libc::MSG_DONTWAIT,
            )
        };
        if n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock {
            Ok(())
        } else {
            Err(invalid())
        }
    }
    struct RuntimeProbeOwner {
        control: Option<(u64, u64)>,
        closed: Option<DataRecord>,
        socket: Option<Arc<UnixStream>>,
        shutdown: Option<PrepaidShutdown>,
        negative_eof: bool,
        negative_drained: usize,
        denied: bool,
        record: Option<usize>,
    }
    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ClosedGateData {
        format_version: u32,
        profile: String,
        base: crate::SourceGateRecord,
        broker_oid: u32,
        schema_contract_sha256: String,
        initial_connect: bool,
        capture_connect_closed: bool,
        capture_sessions_drained: bool,
        release_connect_restored: bool,
    }
    fn validate_closed_record(raw: &[u8], id: Uuid, scan: &SharedJournalBudget) -> Result<()> {
        if raw.is_empty() || raw.len() > 4096 {
            return Err(invalid());
        }
        pay(scan, raw.len() * 64, 32)?;
        let value: ClosedGateData = serde_json::from_slice(raw)?;
        value.base.validate_for(id)?;
        if value.format_version != 2
            || value.profile != "snapshot_c0_d1_v1"
            || value.base.phase() != crate::GatePhase::Closed
            || value.broker_oid == 0
            || !value.initial_connect
            || !value.capture_connect_closed
            || value.capture_sessions_drained
            || value.release_connect_restored
            || value.schema_contract_sha256
                != crate::full_restore::SchemaContract::c0_d1_contract_sha256()?
            || serde_json::to_vec(&value)? != raw
        {
            return Err(invalid());
        }
        Ok(())
    }
    fn connect_acl_denial(raw: &[u8], database: &str, scan: &SharedJournalBudget) -> Result<()> {
        if raw.is_empty() || raw.len() > 4092 {
            return Err(invalid());
        }
        pay(scan, raw.len() * 8 + 1024, 32)?;
        let mut fields = BTreeMap::new();
        let mut at = 0_usize;
        while at < raw.len() {
            let code = raw[at];
            at += 1;
            if code == 0 {
                if at != raw.len() {
                    return Err(invalid());
                }
                break;
            }
            if !b"SVCMDFLR".contains(&code) || fields.contains_key(&code) {
                return Err(invalid());
            }
            let end = raw[at..].iter().position(|b| *b == 0).ok_or_else(invalid)?;
            let value = std::str::from_utf8(&raw[at..at + end]).map_err(|_| invalid())?;
            fields.insert(code, value);
            at += end + 1;
        }
        let message = format!("permission denied for database \"{database}\"");
        if raw.last() != Some(&0)
            || fields.get(&b'V') != Some(&"FATAL")
            || fields.get(&b'C') != Some(&"42501")
            || fields.get(&b'M') != Some(&message.as_str())
            || fields.get(&b'D') != Some(&"User does not have CONNECT privilege.")
            || fields.get(&b'R') != Some(&"CheckMyDatabase")
            || fields.get(&b'F') != Some(&"postinit.c")
        {
            return Err(invalid());
        }
        Ok(())
    }
    fn connect_runtime_socket(deadline: Instant, scan: &SharedJournalBudget) -> Result<UnixStream> {
        fd_gate(2, scan)?;
        pay(scan, 256, 3)?;
        let raw = unsafe {
            libc::socket(
                libc::AF_UNIX,
                libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                0,
            )
        };
        if raw < 0 {
            return Err(invalid());
        }
        let socket = unsafe { UnixStream::from_raw_fd(raw) };
        let mut address = unsafe { std::mem::zeroed::<libc::sockaddr_un>() };
        address.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (to, from) in address
            .sun_path
            .iter_mut()
            .zip(b"/run/knowweave-c4-source-pg/.s.PGSQL.5432")
        {
            *to = *from as libc::c_char;
        }
        let code = unsafe {
            libc::connect(
                raw,
                (&address as *const libc::sockaddr_un).cast(),
                std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
            )
        };
        if code != 0 {
            if !matches!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::EINPROGRESS) | Some(libc::EAGAIN)
            ) {
                return Err(invalid());
            }
            root_socket_poll(&socket, libc::POLLOUT, deadline, &STOP, scan)?;
        }
        let mut error = 0_i32;
        let mut size = std::mem::size_of_val(&error) as libc::socklen_t;
        pay(scan, 0, 1)?;
        if unsafe {
            libc::getsockopt(
                raw,
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                (&mut error as *mut i32).cast(),
                &mut size,
            )
        } != 0
            || error != 0
        {
            return Err(invalid());
        }
        Ok(socket)
    }
    fn require_original_tool_pipe(fd: RawFd, scan: &SharedJournalBudget) -> Result<()> {
        // Qualification is checked on the actual retained FD on every read;
        // the private state bit alone is never evidence of its current flags.
        pay(scan, 0, 2)?;
        require_cloexec(fd)?;
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || flags & libc::O_NONBLOCK == 0 {
            return Err(invalid());
        }
        Ok(())
    }
    fn qualify_original_tool_pipe(fd: RawFd, scan: &SharedJournalBudget) -> Result<()> {
        pay(scan, 0, 5)?;
        mark_cloexec_nonblocking(fd)?;
        require_original_tool_pipe(fd, scan)
    }
    // This one existing host trueChild executes the already-opened PG-image
    // main. Its loader/dependencies resolve in the host namespace, so only
    // the original Root role's actual Catalog CodeFiles may cover them.
    struct RootVersionNative {
        pid: u32,
        start: u64,
        pidfd: ParentPid,
    }
    impl RootVersionNative {
        fn open(
            proc: &ProcView,
            pid: u32,
            parent: &MeasuredProcess,
            main: &File,
            stamp: Stamp,
            binding: &CodeBinding,
            scan: &SharedJournalBudget,
        ) -> Result<Self> {
            fd_gate(4, scan)?;
            parent.recheck(proc, scan)?;
            let start = parse_start(
                &proc.read_relative(&format!("{pid}/stat"), 4096, scan)?,
                pid,
            )?;
            let pidfd = ParentPid::open(pid, start, scan)?;
            let original = Self { pid, start, pidfd };
            original.recheck(proc, parent, main, stamp, binding, scan)?;
            Ok(original)
        }
        fn exited(&self, scan: &SharedJournalBudget) -> Result<bool> {
            pay(scan, 0, 2)?;
            require_cloexec(self.pidfd.fd.as_raw_fd())?;
            let mut p = libc::pollfd {
                fd: self.pidfd.fd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let n = unsafe { libc::poll(&mut p, 1, 0) };
            if n < 0 || p.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
                return Err(invalid());
            }
            Ok(n == 1 && p.revents & libc::POLLIN != 0)
        }
        fn recheck(
            &self,
            proc: &ProcView,
            parent: &MeasuredProcess,
            main: &File,
            stamp: Stamp,
            binding: &CodeBinding,
            scan: &SharedJournalBudget,
        ) -> Result<()> {
            fd_gate(4, scan)?;
            parent.recheck(proc, scan)?;
            self.pidfd.revalidate(scan)?;
            if actual_parent(proc, self.pid, self.start, scan)? != parent.pid
                || process_status(proc, self.pid, 0, scan)? != self.pid
            {
                return Err(invalid());
            }
            let user = TaskNamespace::observe(proc, self.pid, scan)?;
            if !user.object.same_object(&parent.user.object) {
                return Err(invalid());
            }
            drop(user);
            for (leaf, wanted) in [("pid", &parent.pidns), ("mnt", &parent.mntns)] {
                let actual = proc.open_checked_magic(&format!("{}/ns/{leaf}", self.pid), scan)?;
                if actual_object(&actual, scan)? != actual_object(wanted, scan)? {
                    return Err(invalid());
                }
            }
            let root = proc.open_checked_magic(&format!("{}/root", self.pid), scan)?;
            if actual_object(&root, scan)? != parent.root_object {
                return Err(invalid());
            }
            drop(root);
            let current = binding.open_leaf(scan)?;
            if Stamp::observe(&current, scan)? != stamp {
                return Err(invalid());
            }
            drop(current);
            let exe = proc.open_checked_magic(&format!("{}/exe", self.pid), scan)?;
            if Stamp::observe(main, scan)? != stamp || Stamp::observe(&exe, scan)? != stamp {
                return Err(invalid());
            }
            drop(exe);
            let command = proc.read_relative(&format!("{}/cmdline", self.pid), 128, scan)?;
            pay(scan, command.len(), 1)?;
            if command != b"pg_dump\0--version\0" {
                return Err(invalid());
            }
            original_environment(
                proc,
                self.pid,
                &[
                    (
                        "PATH",
                        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
                    ),
                    ("HOME", "/root"),
                    ("LANG", "C"),
                ],
                scan,
            )?;
            for code in &parent.codes[1..] {
                code.recheck(scan)?;
            }
            version_tool_mappings(
                &proc.read_relative(&format!("{}/maps", self.pid), 1024 * 1024, scan)?,
                stamp,
                &parent.codes[1..],
                scan,
            )?;
            if actual_parent(proc, self.pid, self.start, scan)? != parent.pid {
                return Err(invalid());
            }
            parent.recheck(proc, scan)?;
            self.pidfd.revalidate(scan)
        }
    }
    fn version_tool_mappings(
        raw: &[u8],
        main: Stamp,
        allowed: &[Arc<MeasuredCode>],
        scan: &SharedJournalBudget,
    ) -> Result<()> {
        pay(scan, raw.len() * 8 + 4096, 1)?;
        let text = std::str::from_utf8(raw).map_err(|_| invalid())?;
        let (mut main_seen, mut vdso, mut vsys) = (false, false, false);
        for line in text.lines() {
            pay(scan, 0, 8)?;
            let mut words = line.split_ascii_whitespace();
            let mut f = [""; 6];
            for slot in &mut f[..5] {
                *slot = words.next().ok_or_else(invalid)?;
            }
            if f[1].len() != 4 {
                return Err(invalid());
            }
            if f[1].as_bytes()[2] != b'x' {
                continue;
            }
            f[5] = words.next().ok_or_else(invalid)?;
            if words.next().is_some() || f[1].as_bytes()[1] == b'w' || f[1].as_bytes()[3] != b'p' {
                return Err(invalid());
            }
            let (a, b) = f[0].split_once('-').ok_or_else(invalid)?;
            let a = u64::from_str_radix(a, 16).map_err(|_| invalid())?;
            let b = u64::from_str_radix(b, 16).map_err(|_| invalid())?;
            if a == 0 || a >= b {
                return Err(invalid());
            }
            match f[5] {
                "[vdso]" => {
                    if vdso {
                        return Err(invalid());
                    }
                    vdso = true;
                }
                "[vsyscall]" => {
                    if vsys {
                        return Err(invalid());
                    }
                    vsys = true;
                }
                path => {
                    let (major, minor) = f[3].split_once(':').ok_or_else(invalid)?;
                    let major = u32::from_str_radix(major, 16).map_err(|_| invalid())?;
                    let minor = u32::from_str_radix(minor, 16).map_err(|_| invalid())?;
                    let ino = f[4].parse::<u64>().map_err(|_| invalid())?;
                    let offset = u64::from_str_radix(f[2], 16).map_err(|_| invalid())?;
                    if (major, minor, ino)
                        == (libc::major(main.dev), libc::minor(main.dev), main.ino)
                    {
                        // Kernel path spelling is only an alias DATA for the same
                        // actual exe/full-stamp-joined original PG-image File.
                        if !job::path(path) || offset >= main.bytes {
                            return Err(invalid());
                        }
                        main_seen = true;
                    } else {
                        let code = allowed
                            .iter()
                            .find(|code| code.path == path)
                            .ok_or_else(invalid)?;
                        if (major, minor, ino)
                            != (
                                libc::major(code.stamp.dev),
                                libc::minor(code.stamp.dev),
                                code.stamp.ino,
                            )
                            || offset >= code.stamp.bytes
                        {
                            return Err(invalid());
                        }
                    }
                }
            }
        }
        if !main_seen {
            return Err(invalid());
        }
        Ok(())
    }

    struct FixedTool {
        child: Option<Child>,
        stdout: Option<std::process::ChildStdout>,
        stderr: Option<std::process::ChildStderr>,
        out: Vec<u8>,
        err: Vec<u8>,
        out_eof: bool,
        err_eof: bool,
        out_nonblocking: bool,
        err_nonblocking: bool,
        status: Option<std::process::ExitStatus>,
        native: Option<RootVersionNative>,
        exe: Option<File>,
        binding: Option<CodeBinding>,
        stamp: Option<Stamp>,
    }

    // DATA-only validation is the existing destination historical protection
    // algorithm, joined here to the Root's independently read original source
    // bytes. It neither opens a ctx nor calls a destination publication owner.
    fn validate_m5_source_data(
        files: &BTreeMap<String, Vec<u8>>,
        group: &registry::SourceGroup,
        generation: &registry::Generation,
        manifest: &crate::BackupManifestV1,
        index: &[u8],
        pin_identity: (u64, u64),
        scan: &SharedJournalBudget,
    ) -> Result<()> {
        let total = files
            .values()
            .try_fold(0_usize, |n, v| n.checked_add(v.len()).ok_or_else(invalid))?;
        pay(
            scan,
            total
                .checked_mul(128)
                .and_then(|n| n.checked_add(index.len().checked_mul(8)?))
                .and_then(|n| n.checked_add(manifest.files.len().checked_mul(1024)?))
                .ok_or_else(invalid)?,
            files.len() * 64 + manifest.files.len() * 4,
        )?;
        let manifest_sha256 = crate::digest(&manifest.canonical_bytes()?);
        let released_raw = files.get("source-released.json").ok_or_else(invalid)?;
        let released = crate::source::c0_d1::validate_full_c_released_record(
            released_raw,
            manifest.backup_id,
            &manifest_sha256,
        )?;
        released.validate_for(manifest.backup_id)?;
        if released.phase() != crate::GatePhase::Released
            || released.pinned_manifest_sha256() != Some(&manifest_sha256)
            || serde_json::to_value(&released)?["dump_and_index_sha256"] != manifest_sha256
        {
            return Err(invalid());
        }
        let snapshot: crate::destination::SnapshotRecord = parse(
            files.get("protection-snapshot.json").ok_or_else(invalid)?,
            RECORD_CAP,
            scan,
        )?;
        if snapshot.format_version != 1
            || snapshot.capability != "protection_snapshot_v1"
            || snapshot.source_group_id != group.group_id
            || snapshot.registry_generation_sha256
                != crate::digest(&files["source-generation.json"])
            || snapshot.protected_digest_count > 100_000
            || snapshot.scan_bytes > 512 * 1024 * 1024
            || snapshot.records.len() > 100_000
            || snapshot.records.windows(2).any(|w| w[0].path >= w[1].path)
        {
            return Err(invalid());
        }
        pay(
            scan,
            snapshot
                .records
                .len()
                .checked_mul(1024)
                .ok_or_else(invalid)?,
            snapshot.records.len() * 32,
        )?;
        let mut total = 0_u64;
        for item in &snapshot.records {
            if !relative_metadata_path(&item.path)
                || !crate::valid_digest(&item.sha256)
                || item.size > crate::MAX_ASSET_INDEX_BYTES as u64
            {
                return Err(invalid());
            }
            total = total.checked_add(item.size).ok_or_else(invalid)?;
        }
        if total > snapshot.scan_bytes {
            return Err(invalid());
        }
        validate_protection_chain(
            manifest.backup_id,
            &manifest_sha256,
            files,
            group,
            generation,
            (manifest, index),
            &snapshot,
        )?;
        let retained: serde_json::Value =
            serde_json::from_slice(&files["protection-retained.json"])?;
        if retained["sealed_dev"].as_u64() != Some(pin_identity.0)
            || retained["sealed_ino"].as_u64() != Some(pin_identity.1)
        {
            return Err(invalid());
        }
        Ok(())
    }

    fn relative_metadata_path(path: &str) -> bool {
        !path.is_empty()
            && !path.contains(['\\', '\0', ':'])
            && path
                .split('/')
                .all(|p| !p.is_empty() && p != "." && p != "..")
    }

    fn snapshot_ref(
        snapshot: &crate::destination::SnapshotRecord,
        path: &str,
        bytes: &[u8],
    ) -> Result<()> {
        if !snapshot.records.iter().any(|r| {
            r.path == path && r.size == bytes.len() as u64 && r.sha256 == crate::digest(bytes)
        }) {
            return Err(BackupError::Invalid(
                "captured snapshot reference missing or changed",
            ));
        }
        Ok(())
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct HistoricalProtection {
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
    fn validate_protection_chain(
        backup_id: Uuid,
        manifest_sha256: &str,
        files: &BTreeMap<String, Vec<u8>>,
        group: &registry::SourceGroup,
        generation: &registry::Generation,
        payload: (&crate::BackupManifestV1, &[u8]),
        snapshot: &crate::destination::SnapshotRecord,
    ) -> Result<()> {
        let (manifest, index) = payload;
        let mut predecessor = None;
        let mut capture_generation = None;
        for (n, (name, original, state, extras)) in [
            (
                "protection-pending.json",
                "00-pending.json",
                "capture_pending",
                vec!["roots"],
            ),
            (
                "protection-catalog.json",
                "10-catalog.json",
                "catalog_durable",
                vec!["asset_index", "logical_asset_count"],
            ),
            (
                "protection-retained.json",
                "20-retained.json",
                "retained",
                vec![
                    "asset_index_sha256",
                    "manifest_sha256",
                    "sealed_name",
                    "sealed_dev",
                    "sealed_ino",
                ],
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let raw = &files[name];
            // Value parsing is for historical fields only; require exact keys and original serialization order is preserved in hashes.
            let parsed: HistoricalProtection = registry::canonical_record(raw, 16384)?;
            let value = serde_json::to_value(parsed)?;
            let object = value
                .as_object()
                .ok_or(BackupError::Invalid("captured protection record"))?;
            let mut keys = vec![
                "format_version",
                "capability",
                "deployment_id",
                "group_id",
                "backup_id",
                "registry_generation",
                "registry_generation_sha256",
                "source_binding_sha256",
                "application_commit",
                "application_build_sha256",
                "state",
                "previous_sha256",
            ];
            keys.extend(extras);
            if object.len() != keys.len()
                || keys.iter().any(|k| !object.contains_key(*k))
                || value["format_version"] != 1
                || value["capability"] != "source_protection_v1"
                || value["deployment_id"] != group.deployment_id
                || value["group_id"] != group.group_id
                || value["backup_id"] != backup_id.to_string()
                || value["state"] != state
                || value["source_binding_sha256"] != group.source_binding_sha256
                || value["application_commit"] != group.application_commit
                || value["application_build_sha256"] != group.application_build_sha256
                || value["previous_sha256"]
                    != predecessor
                        .clone()
                        .map(serde_json::Value::String)
                        .unwrap_or(serde_json::Value::Null)
            {
                return Err(BackupError::Invalid("captured protection chain"));
            }
            let gen_number = value["registry_generation"]
                .as_u64()
                .filter(|v| *v > 0 && *v <= generation.generation)
                .ok_or(BackupError::Invalid("captured protection generation"))?;
            let gen_hash = value["registry_generation_sha256"]
                .as_str()
                .filter(|v| crate::valid_digest(v))
                .ok_or(BackupError::Invalid("captured protection generation hash"))?;
            if capture_generation
                .as_ref()
                .is_some_and(|v| v != &(gen_number, gen_hash.to_owned()))
                || !snapshot.records.iter().any(|r| {
                    r.path == format!("registry/generations/{gen_number:020}.json")
                        && r.sha256 == gen_hash
                })
            {
                return Err(BackupError::Invalid(
                    "captured protection generation linkage",
                ));
            }
            capture_generation = Some((gen_number, gen_hash.to_owned()));
            match n {
                0 if value["roots"] == serde_json::to_value(&group.roots)? => (),
                1 if value["asset_index"]
                    == serde_json::to_value(
                        manifest
                            .files
                            .iter()
                            .find(|r| r.path == "asset-index.json")
                            .ok_or(BackupError::Invalid("captured index absent"))?,
                    )?
                    && value["logical_asset_count"] == manifest.logical_asset_count => {}
                2 if value["asset_index_sha256"] == crate::digest(index)
                    && value["manifest_sha256"] == manifest_sha256
                    && value["sealed_name"] == format!("{}.sealed", backup_id)
                    && value["sealed_dev"].as_u64().is_some_and(|v| v > 0)
                    && value["sealed_ino"].as_u64().is_some_and(|v| v > 0) => {}
                _ => return Err(BackupError::Invalid("captured protection stage fields")),
            }
            snapshot_ref(
                snapshot,
                &format!("protection/{}/{original}", backup_id),
                raw,
            )?;
            predecessor = Some(crate::digest(raw));
        }
        snapshot_ref(
            snapshot,
            &format!("protection/{}/asset-index.json", backup_id),
            index,
        )?;
        snapshot_ref(
            snapshot,
            &format!("control/{}.control/released.json", backup_id),
            &files["source-released.json"],
        )?;
        snapshot_ref(
            snapshot,
            "registry/authority.json",
            &files["source-authority.json"],
        )?;
        snapshot_ref(
            snapshot,
            &format!("registry/groups/{}.json", group.group_id),
            &files["source-group.json"],
        )?;
        for id in group.roots.values() {
            snapshot_ref(
                snapshot,
                &format!("registry/roots/{id}.json"),
                &files[&format!("roots/{id}.json")],
            )?;
        }
        let unique = manifest
            .files
            .iter()
            .filter(|r| r.path.starts_with("assets/"))
            .count() as u64;
        if snapshot.protected_digest_count < unique {
            return Err(BackupError::Invalid("captured protected digest count"));
        }
        Ok(())
    }

    enum ArtifactDirectory<'a> {
        Root(&'a BackupDir),
        Child(BackupDir),
    }
    impl ArtifactDirectory<'_> {
        fn directory(&self) -> &BackupDir {
            match self {
                Self::Root(d) => d,
                Self::Child(d) => d,
            }
        }
    }
    fn artifact_directory<'a>(
        root: &'a BackupDir,
        path: &str,
        scan: &SharedJournalBudget,
    ) -> Result<ArtifactDirectory<'a>> {
        pay(scan, path.len() * 8 + 1024, 0)?;
        pay(scan, 0, path.split('/').count() * 5 + 4)?;
        if !path.is_empty()
            && (!crate::source::snapshot_effects::wire::path(path) || path.split('/').count() > 16)
        {
            return Err(invalid());
        }
        root.require_sealed_directory()?;
        let mut current = ArtifactDirectory::Root(root);
        if !path.is_empty() {
            for part in path.split('/') {
                fd_gate(2, scan)?;
                pay(scan, part.len() + 128, 3)?;
                let child = current.directory().open_dir(part)?;
                child.require_sealed_directory()?;
                current = ArtifactDirectory::Child(child);
            }
        }
        Ok(current)
    }
    fn artifact_parent<'a>(
        root: &'a BackupDir,
        path: &str,
        scan: &SharedJournalBudget,
    ) -> Result<(ArtifactDirectory<'a>, String)> {
        if !crate::source::snapshot_effects::wire::path(path) || path.split('/').count() > 16 {
            return Err(invalid());
        }
        let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
        let parent = artifact_directory(root, parent, scan)?;
        pay(scan, name.len(), 1)?;
        Ok((parent, name.into()))
    }
    fn artifact_bytes(
        root: &BackupDir,
        path: &str,
        cap: usize,
        deadline: Instant,
        scan: &SharedJournalBudget,
    ) -> Result<Vec<u8>> {
        root_io_boundary(deadline, &STOP)?;
        let (parent, name) = artifact_parent(root, path, scan)?;
        pay(scan, name.len() + 1, 4)?;
        fd_gate(2, scan)?;
        let mut file = parent.directory().open_file(&name)?;
        let stamp = Stamp::observe(&file, scan)?;
        if !stamp.root_file(cap as u64) || stamp.gid != 0 || stamp.mode & 0o7777 != 0o400 {
            return Err(invalid());
        }
        let raw = read_exact_file(&mut file, stamp.bytes, scan)?;
        pay(scan, name.len() + 1, 3)?;
        if Stamp::observe(&file, scan)? != stamp {
            return Err(invalid());
        }
        drop(file);
        if Stamp::observe(&parent.directory().open_file(&name)?, scan)? != stamp {
            return Err(invalid());
        }
        root_io_boundary(deadline, &STOP)?;
        Ok(raw)
    }
    fn exact_artifact_tree(
        root: &BackupDir,
        prefix: &str,
        records: &[crate::FileRecord],
        depth: usize,
        deadline: Instant,
        scan: &SharedJournalBudget,
    ) -> Result<()> {
        root_io_boundary(deadline, &STOP)?;
        if !prefix.is_empty() || depth != 0 || records.is_empty() || records.len() > 100_000 {
            return Err(invalid());
        }
        let path_bytes = records.iter().try_fold(0_usize, |n, r| {
            n.checked_add(r.path.len()).ok_or_else(invalid)
        })?;
        pay(
            scan,
            path_bytes
                .checked_mul(32)
                .and_then(|v| v.checked_add(records.len() * 4096))
                .ok_or_else(invalid)?,
            records.len() * 32 + 3,
        )?;
        if records.windows(2).any(|w| w[0].path >= w[1].path)
            || records.iter().any(|r| {
                !crate::source::snapshot_effects::wire::path(&r.path)
                    || r.path.split('/').count() > 16
                    || r.size == 0
                    || !crate::valid_digest(&r.sha256)
            })
        {
            return Err(invalid());
        }
        let mut directories: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut files = BTreeMap::new();
        for record in records {
            files.insert(record.path.as_str(), record);
            let mut parent = String::new();
            let parts: Vec<_> = record.path.split('/').collect();
            for (part_index, part) in parts.iter().enumerate() {
                directories
                    .entry(parent.clone())
                    .or_default()
                    .insert((*part).into());
                if part_index + 1 < parts.len() {
                    if !parent.is_empty() {
                        parent.push('/');
                    }
                    parent.push_str(part);
                }
            }
        }
        // Each directory reader finishes before the next one is opened. The
        // original sealed bundle anchor stays held; directory identities and
        // exact rosters are checked twice, including after all file reads.
        let mut observed = BTreeMap::new();
        for (relative, names) in &directories {
            root_io_boundary(deadline, &STOP)?;
            let current = artifact_directory(root, relative, scan)?;
            let directory = current.directory();
            let identity = directory.identity()?;
            let before = roster(directory, names.len(), scan)?;
            if before.iter().ne(names.iter()) {
                return Err(invalid());
            }
            for name in &before {
                let path = if relative.is_empty() {
                    name.clone()
                } else {
                    format!("{relative}/{name}")
                };
                pay(scan, path.len() + 128, 4)?;
                if let Some(record) = files.get(path.as_str()) {
                    if directory.kind(name)? != learning_assets::backup_fs::BackupEntryKind::File {
                        return Err(invalid());
                    }
                    fd_gate(2, scan)?;
                    let mut file = directory.open_file(name)?;
                    let stamp = Stamp::observe(&file, scan)?;
                    if !stamp.root_file(u64::MAX)
                        || stamp.gid != 0
                        || stamp.mode & 0o7777 != 0o400
                        || stamp.bytes != record.size
                    {
                        return Err(invalid());
                    }
                    let mut hash = Sha256::new();
                    let mut used = 0_u64;
                    let mut buffer = [0_u8; 65536];
                    loop {
                        root_io_boundary(deadline, &STOP)?;
                        let want = usize::try_from(
                            (record.size - used)
                                .saturating_add(1)
                                .min(buffer.len() as u64),
                        )
                        .map_err(|_| invalid())?;
                        pay(scan, want * 2, 1)?;
                        let n = match file.read(&mut buffer[..want]) {
                            Ok(n) => n,
                            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                            _ => return Err(invalid()),
                        };
                        if n == 0 {
                            break;
                        }
                        used = used
                            .checked_add(n as u64)
                            .filter(|n| *n <= record.size)
                            .ok_or_else(invalid)?;
                        hash.update(&buffer[..n]);
                    }
                    if used != record.size
                        || hex::encode(hash.finalize()) != record.sha256
                        || Stamp::observe(&file, scan)? != stamp
                    {
                        return Err(invalid());
                    }
                    drop(file);
                    pay(scan, name.len() + 128, 3)?;
                    if Stamp::observe(&directory.open_file(name)?, scan)? != stamp {
                        return Err(invalid());
                    }
                } else if directory.kind(name)?
                    != learning_assets::backup_fs::BackupEntryKind::Directory
                    || !directories.contains_key(&path)
                {
                    return Err(invalid());
                }
            }
            if directory.identity()? != identity || roster(directory, names.len(), scan)? != before
            {
                return Err(invalid());
            }
            observed.insert(relative.clone(), identity);
        }
        for (relative, names) in &directories {
            root_io_boundary(deadline, &STOP)?;
            let current = artifact_directory(root, relative, scan)?;
            if observed.get(relative) != Some(&current.directory().identity()?)
                || roster(current.directory(), names.len(), scan)?
                    .iter()
                    .ne(names.iter())
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
    fn exec_exit_data(v: &serde_json::Value) -> Result<(bool, Option<i64>)> {
        let running = jbool(v, "Running")?;
        let value = v.get("ExitCode").ok_or_else(invalid)?;
        let exit = match value {
            serde_json::Value::Null => None,
            v => Some(v.as_i64().ok_or_else(invalid)?),
        };
        if running && exit.is_some() {
            return Err(invalid());
        }
        Ok((running, exit))
    }
    fn prepay_original_prefix(
        files: &BTreeMap<String, Vec<u8>>,
        descriptor: &[u8],
        scan: &SharedJournalBudget,
    ) -> Result<()> {
        let work = files.values().try_fold(descriptor.len(), |n, v| {
            n.checked_add(v.len()).ok_or_else(invalid)
        })?;
        // Pay byte work before the fixed-stack lexical pass. Each lexical node
        // is charged before it is visited; all remaining repeated pure passes
        // are then paid before unique_json can allocate any Value/Vec/Map.
        pay(
            scan,
            work.checked_mul(64).ok_or_else(invalid)?,
            files.len() * 4 + 4,
        )?;
        let mut nodes = prefix_json_nodes(descriptor, scan)?;
        for (name, raw) in files {
            if name == "journal/epoch.header" {
                continue;
            }
            if !name.starts_with("journal/")
                || !name.ends_with(".frame")
                || !(120..=FRAME_CAP).contains(&raw.len())
            {
                return Err(invalid());
            }
            nodes = nodes
                .checked_add(prefix_json_nodes(&raw[120..], scan)?)
                .ok_or_else(invalid)?;
        }
        pay(
            scan,
            0,
            nodes
                .checked_mul(15)
                .and_then(|n| n.checked_add(files.len() * 16 + 32))
                .ok_or_else(invalid)?,
        )
    }
    fn prefix_json_nodes(raw: &[u8], scan: &SharedJournalBudget) -> Result<usize> {
        // No recursive descent, String decoding, Vec, Map or input-sized
        // allocation. Syntax authority remains the original pure decoder.
        let mut stack = [0_u8; 128];
        let mut depth = 0_usize;
        let mut index = 0_usize;
        let mut nodes = 0_usize;
        while index < raw.len() {
            let byte = raw[index];
            match byte {
                b' ' | b'\t' | b'\r' | b'\n' | b',' | b':' => index += 1,
                b'{' | b'[' => {
                    pay(scan, 0, 1)?;
                    nodes += 1;
                    if depth == stack.len() {
                        return Err(invalid());
                    }
                    stack[depth] = byte;
                    depth += 1;
                    index += 1;
                }
                b'}' | b']' => {
                    if depth == 0 || stack[depth - 1] != if byte == b'}' { b'{' } else { b'[' } {
                        return Err(invalid());
                    }
                    depth -= 1;
                    index += 1;
                }
                b'"' => {
                    pay(scan, 0, 1)?;
                    nodes += 1;
                    index += 1;
                    let mut ended = false;
                    while index < raw.len() {
                        match raw[index] {
                            b'"' => {
                                index += 1;
                                ended = true;
                                break;
                            }
                            b'\\' => {
                                index = index
                                    .checked_add(2)
                                    .filter(|n| *n <= raw.len())
                                    .ok_or_else(invalid)?;
                            }
                            n if n < 0x20 => return Err(invalid()),
                            _ => index += 1,
                        }
                    }
                    if !ended {
                        return Err(invalid());
                    }
                }
                b'-' | b'0'..=b'9' | b't' | b'f' | b'n' => {
                    pay(scan, 0, 1)?;
                    nodes += 1;
                    index += 1;
                    while index < raw.len()
                        && !matches!(
                            raw[index],
                            b' ' | b'\t' | b'\r' | b'\n' | b',' | b']' | b'}'
                        )
                    {
                        if matches!(raw[index], b'{' | b'[' | b'"' | b':' | b'\\') {
                            return Err(invalid());
                        }
                        index += 1;
                    }
                }
                _ => return Err(invalid()),
            }
        }
        if depth != 0 || nodes == 0 {
            return Err(invalid());
        }
        Ok(nodes)
    }
    fn original_owner_deadline(
        total: Instant,
        ledger: Instant,
        phase: Option<Instant>,
        negative: Option<Instant>,
    ) -> Instant {
        total
            .min(ledger)
            .min(phase.unwrap_or(total))
            .min(negative.unwrap_or(total))
    }
    fn capture_pid_for_phase(
        step: usize,
        natural: bool,
        measured: Option<u32>,
        bootstrap: Option<(u32, u64)>,
    ) -> Result<u32> {
        if natural {
            return if step >= 15 { Ok(0) } else { Err(invalid()) };
        }
        if let Some(pid) = measured {
            return if step >= 4 && pid > 0 {
                Ok(pid)
            } else {
                Err(invalid())
            };
        }
        match bootstrap {
            Some((pid, start)) if (2..=3).contains(&step) && pid > 0 && start > 0 => Ok(pid),
            _ => Err(invalid()),
        }
    }
    #[cfg(test)]
    mod original_runtime_data_tests {
        use super::*;
        use std::io::Write;
        #[test]
        fn bootstrap_pid_data_expires_at_final_init_boundary() {
            assert_eq!(
                capture_pid_for_phase(2, false, None, Some((77, 900))).unwrap(),
                77
            );
            assert_eq!(
                capture_pid_for_phase(3, false, None, Some((77, 900))).unwrap(),
                77
            );
            assert!(capture_pid_for_phase(4, false, None, Some((77, 900))).is_err());
            assert!(capture_pid_for_phase(14, false, None, Some((77, 900))).is_err());
            assert!(capture_pid_for_phase(3, false, Some(77), Some((77, 900))).is_err());
            assert!(capture_pid_for_phase(2, false, None, Some((77, 0))).is_err());
            assert_eq!(
                capture_pid_for_phase(4, false, Some(77), Some((77, 900))).unwrap(),
                77
            );
            assert!(capture_pid_for_phase(14, true, Some(77), Some((77, 900))).is_err());
            assert_eq!(
                capture_pid_for_phase(15, true, None, Some((77, 900))).unwrap(),
                0
            );
        }
        #[test]
        fn terminal_deadline_survives_ledger_pending_clear_and_elapsed_wait() {
            let now = Instant::now();
            let total = now + Duration::from_secs(1200);
            let phase = now + Duration::from_millis(10);
            let mut ledger = RootControlLedger::data_only(
                SharedJournalBudget::default(),
                "11111111-1111-4111-8111-111111111111",
                &"a".repeat(64),
                total,
            )
            .unwrap();
            ledger.pending = Some(phase);
            assert_eq!(
                original_owner_deadline(total, ledger.deadline(), Some(phase), None),
                phase
            );
            ledger.pending = None;
            assert_eq!(
                original_owner_deadline(total, ledger.deadline(), Some(phase), None),
                phase
            );
            std::thread::sleep(Duration::from_millis(15));
            let selected =
                original_owner_deadline(total, ledger.deadline(), Some(phase), Some(total));
            assert!(Instant::now() >= selected);
            assert_eq!(selected, phase);
        }
        #[test]
        fn dense_prefix_is_entry_refused_before_pure_decoder() {
            let meter = SharedJournalBudget::default();
            let mut files = BTreeMap::new();
            let mut frame = vec![0; 120];
            frame.extend_from_slice(format!("[{}]", vec!["0"; 8000].join(",")).as_bytes());
            files.insert("journal/00000000000000000001.frame".into(), frame);
            assert!(prepay_original_prefix(&files, b"{}", &meter).is_err());
        }
        #[test]
        fn long_prefix_string_is_not_billed_as_thousands_of_nodes() {
            let meter = SharedJournalBudget::default();
            let mut files = BTreeMap::new();
            let mut frame = vec![0; 120];
            frame.extend_from_slice(format!("\"{}\"", "a".repeat(16 * 1024)).as_bytes());
            files.insert("journal/00000000000000000001.frame".into(), frame);
            assert!(prepay_original_prefix(&files, b"{}", &meter).is_ok());
            let deep = format!("{}0{}", "[".repeat(129), "]".repeat(129));
            assert!(prefix_json_nodes(deep.as_bytes(), &meter).is_err());
        }
        #[test]
        fn exec_exit_data_keeps_live_null_separate_from_terminal_zero() {
            assert_eq!(
                exec_exit_data(&serde_json::json!({"Running":true,"ExitCode":null})).unwrap(),
                (true, None)
            );
            assert_eq!(
                exec_exit_data(&serde_json::json!({"Running":false,"ExitCode":0})).unwrap(),
                (false, Some(0))
            );
            assert_eq!(
                exec_exit_data(&serde_json::json!({"Running":false,"ExitCode":null})).unwrap(),
                (false, None)
            );
            assert!(exec_exit_data(&serde_json::json!({"Running":true,"ExitCode":0})).is_err());
            assert!(exec_exit_data(&serde_json::json!({"Running":false,"ExitCode":"0"})).is_err());
        }
        #[test]
        fn docker_api_json_rejects_duplicate_fields_recursively() {
            let meter = SharedJournalBudget::default();
            assert!(api_json(br#"{"a":{"Id":"x","Id":"y"}}"#, 4096, &meter).is_err());
            assert!(api_json(br#"{"a":1,"a":2}"#, 4096, &meter).is_err());
            assert!(api_json(br#"{"a":[{"Id":"x"}]}"#, 4096, &meter).is_ok());
        }
        #[test]
        fn runtime_acl_diagnostic_rejects_auth_and_generic_permission_errors() {
            let meter = SharedJournalBudget::default();
            let db = "learning_backup_c4_task3_11111111-1111-4111-8111-111111111111";
            let message = format!("permission denied for database \"{db}\"");
            let raw=format!("SFATAL\0VFATAL\0C42501\0M{message}\0DUser does not have CONNECT privilege.\0Fpostinit.c\0L456\0RCheckMyDatabase\0\0").into_bytes();
            assert!(connect_acl_denial(&raw, db, &meter).is_ok());
            let auth = String::from_utf8(raw.clone())
                .unwrap()
                .replace("42501", "28P01")
                .into_bytes();
            assert!(connect_acl_denial(&auth, db, &meter).is_err());
            let wrong = String::from_utf8(raw.clone())
                .unwrap()
                .replace("CheckMyDatabase", "ClientAuthentication")
                .into_bytes();
            assert!(connect_acl_denial(&wrong, db, &meter).is_err());
            let missing = String::from_utf8(raw.clone())
                .unwrap()
                .replace("DUser does not have CONNECT privilege.\0", "")
                .into_bytes();
            assert!(connect_acl_denial(&missing, db, &meter).is_err());
            let mut duplicate = raw[..raw.len() - 1].to_vec();
            duplicate.extend_from_slice(b"C42501\0\0");
            assert!(connect_acl_denial(&duplicate, db, &meter).is_err());
        }
        #[test]
        fn code_ancestor_data_rejects_writable_or_non_root_parents() {
            let base = Stamp {
                dev: 1,
                ino: 2,
                uid: 0,
                gid: 0,
                mode: libc::S_IFDIR | 0o755,
                links: 2,
                bytes: 4096,
                mtime_ns: 1,
                ctime_ns: 1,
            };
            assert!(code_directory(base).is_ok());
            let mut bad = base;
            bad.uid = 1000;
            assert!(code_directory(bad).is_err());
            bad = base;
            bad.mode |= 0o020;
            assert!(code_directory(bad).is_err());
            bad = base;
            bad.mode = libc::S_IFREG | 0o644;
            assert!(code_directory(bad).is_err());
        }
        #[test]
        fn original_pipe_qualification_rechecks_actual_nonblocking_flags() {
            let (read, write) = UnixStream::pair().unwrap();
            let meter = SharedJournalBudget::default();
            assert!(require_original_tool_pipe(read.as_raw_fd(), &meter).is_err());
            qualify_original_tool_pipe(read.as_raw_fd(), &meter).unwrap();
            require_original_tool_pipe(read.as_raw_fd(), &meter).unwrap();
            let flags = unsafe { libc::fcntl(read.as_raw_fd(), libc::F_GETFL) };
            assert!(flags >= 0);
            assert_eq!(
                unsafe { libc::fcntl(read.as_raw_fd(), libc::F_SETFL, flags & !libc::O_NONBLOCK) },
                0
            );
            assert!(require_original_tool_pipe(read.as_raw_fd(), &meter).is_err());
            assert!(read.as_raw_fd() >= 0);
            drop(write);
        }
        #[test]
        fn fixed_helper_argv_rejects_extra_options_and_wrong_database() {
            let meter = SharedJournalBudget::default();
            let db = "learning_backup_c4_task3_11111111-1111-4111-8111-111111111111";
            let version = b"/usr/lib/postgresql/18/bin/pg_dump\0--version\0";
            assert_eq!(
                original_pg_helper_recipe(version, db, &meter).unwrap(),
                OriginalPgHelperKind::DumpVersion
            );
            let args = [
                "/usr/lib/postgresql/18/bin/pg_dump",
                "--format=custom",
                "--no-password",
                "--lock-wait-timeout=5000",
                "--host=/var/run/postgresql",
                "--port=5432",
                "--username=learning_admin",
            ];
            let raw = format!("{}\0--dbname={db}\0", args.join("\0")).into_bytes();
            assert_eq!(
                original_pg_helper_recipe(&raw, db, &meter).unwrap(),
                OriginalPgHelperKind::Dump
            );
            let mut extra = raw.clone();
            extra.extend_from_slice(b"--verbose\0");
            assert!(original_pg_helper_recipe(&extra, db, &meter).is_err());
            let many = vec![0_u8; 8192];
            assert!(original_pg_helper_recipe(&many, db, &meter).is_err());
            assert!(
                original_pg_helper_recipe(
                    &raw,
                    "learning_backup_c4_task3_22222222-2222-4222-8222-222222222222",
                    &meter
                )
                .is_err()
            );
            assert!(original_pg_helper_recipe(&raw[..raw.len() - 1], db, &meter).is_err());
        }
        #[test]
        fn helper_sql_shape_cannot_become_an_arbitrary_statement() {
            let sql = "SELECT pg_catalog.pg_backend_pid()::text||'|'||backend_start::text FROM pg_catalog.pg_stat_activity WHERE pid=pg_catalog.pg_backend_pid();BEGIN READ ONLY; SET LOCAL statement_timeout = '5000ms'; SELECT current_user::text || '|' || pg_catalog.current_database() || '|' || d.oid::text || '|' || pcs.system_identifier::text FROM pg_catalog.pg_database d CROSS JOIN pg_catalog.pg_control_system() pcs WHERE d.datname=pg_catalog.current_database(); SELECT pid, database::bigint, classid::bigint, objid::bigint, objsubid, mode, granted FROM pg_catalog.pg_locks WHERE locktype='advisory' AND (classid::bigint,objid::bigint) IN ((0,4294967295),(12,34)); ROLLBACK;";
            original_observer_sql_shape(sql).unwrap();
            for changed in [
                sql.replace("(12,34)", "(012,34)"),
                sql.replace("4294967295", "4294967296"),
                sql.replace("ROLLBACK;", "COMMIT;"),
                format!("{sql}SELECT 1;"),
            ] {
                assert!(original_observer_sql_shape(&changed).is_err());
            }
        }
        #[test]
        fn helper_executable_closure_rejects_anonymous_and_unlisted_main() {
            let meter = SharedJournalBudget::default();
            let main = "/usr/lib/postgresql/18/bin/psql";
            let raw = format!(
                "1000-2000 r-xp 00000000 01:02 3 {main}\n2000-3000 r-xp 00000000 01:02 4 /usr/lib/x86_64-linux-gnu/libc.so.6\n3000-4000 r-xp 00000000 00:00 0 [vdso]\n"
            );
            assert_eq!(
                original_helper_paths(raw.as_bytes(), main, &meter)
                    .unwrap()
                    .len(),
                2
            );
            assert!(
                original_helper_paths(raw.replace(main, "/tmp/psql").as_bytes(), main, &meter)
                    .is_err()
            );
            assert!(
                original_helper_paths(raw.replace("[vdso]", "[anon:jit]").as_bytes(), main, &meter)
                    .is_err()
            );
            assert!(
                original_helper_paths(raw.replace("r-xp", "rwxp").as_bytes(), main, &meter)
                    .is_err()
            );
            let excess = format!(
                "1000-2000 r-xp 00000000 01:02 3 {main} {}\n",
                "extra ".repeat(8192)
            );
            assert!(original_helper_paths(excess.as_bytes(), main, &meter).is_err());
        }
        #[test]
        fn negative_exact_inspection_keeps_its_original_parser_bytes() {
            let (mut writer, reader) = UnixStream::pair().unwrap();
            mark_cloexec_nonblocking(reader.as_raw_fd()).unwrap();
            let reader = Arc::new(reader);
            let meter = SharedJournalBudget::default();
            let deadline = Instant::now() + Duration::from_secs(1);
            let shutdown = PrepaidShutdown::bind(
                Arc::clone(&reader),
                Arc::new(std::sync::atomic::AtomicU8::new(0)),
            );
            let mut owner = OriginalHttpOwner {
                socket: Arc::clone(&reader),
                shutdown,
                purpose: HttpPurpose::NegativeExactExecInspect,
                negative_eof: false,
                negative_drained: 0,
                path: String::new(),
                request: Vec::new(),
                written: 0,
                header: Vec::new(),
                head: None,
                body: Vec::new(),
                line: Vec::new(),
                phase: HttpPhase::Head,
                remaining: 0,
                delimiter: 0,
                cap: 4096,
                deadline,
                connect_started: false,
                peer_checked: false,
                upgrade: false,
            };
            writer.write_all(b"H").unwrap();
            negative_http_drain(&mut owner, deadline, &meter).unwrap();
            let mut byte = 0_u8;
            assert_eq!(
                unsafe {
                    libc::recv(
                        reader.as_raw_fd(),
                        (&mut byte as *mut u8).cast(),
                        1,
                        libc::MSG_PEEK | libc::MSG_DONTWAIT,
                    )
                },
                1
            );
            assert_eq!(byte, b'H');
            assert_eq!(owner.negative_drained, 0);
            assert!(!owner.negative_eof);
            owner.purpose = HttpPurpose::Positive;
            negative_http_drain(&mut owner, deadline, &meter).unwrap();
            assert_eq!(owner.negative_drained, 1);
        }
        #[test]
        fn terminal_capture_stat_accepts_only_same_start_dead_task_data() {
            let raw = format!("77 (capture) Z 1 {} 900 0\n", vec!["0"; 17].join(" "));
            terminal_capture_stat_data(raw.as_bytes(), 77, 900).unwrap();
            assert!(terminal_capture_stat_data(raw.as_bytes(), 77, 901).is_err());
            assert!(terminal_capture_stat_data(raw.as_bytes(), 78, 900).is_err());
            assert!(
                terminal_capture_stat_data(raw.replace(") Z ", ") R ").as_bytes(), 77, 900)
                    .is_err()
            );
        }
        #[test]
        fn host_version_maps_require_original_main_and_reject_unknown_libraries() {
            let stamp = Stamp {
                dev: libc::makedev(1, 2),
                ino: 3,
                uid: 0,
                gid: 0,
                mode: libc::S_IFREG | 0o755,
                links: 1,
                bytes: 4096,
                mtime_ns: 1,
                ctime_ns: 1,
            };
            let meter = SharedJournalBudget::default();
            let raw=b"1000-2000 r-xp 00000000 01:02 3 /kernel/alias/pg_dump\n2000-3000 r-xp 00000000 00:00 0 [vdso]\n";
            version_tool_mappings(raw, stamp, &[], &meter).unwrap();
            assert!(
                version_tool_mappings(
                    &[
                        raw.as_slice(),
                        b"3000-4000 r-xp 00000000 01:02 4 /usr/lib/unknown.so.1\n"
                    ]
                    .concat(),
                    stamp,
                    &[],
                    &meter
                )
                .is_err()
            );
            assert!(
                version_tool_mappings(
                    b"1000-2000 r-xp 00000000 01:02 9 /kernel/alias/pg_dump\n",
                    stamp,
                    &[],
                    &meter
                )
                .is_err()
            );
            assert!(
                version_tool_mappings(
                    b"1000-2000 rwxp 00000000 01:02 3 /kernel/alias/pg_dump\n",
                    stamp,
                    &[],
                    &meter
                )
                .is_err()
            );
        }
        fn mux_frame(kind: u8, body: &[u8]) -> Vec<u8> {
            let mut out = vec![kind, 0, 0, 0];
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            out.extend_from_slice(body);
            out
        }
        #[test]
        fn mux_reads_stdout_and_stderr_to_real_socket_eof() {
            let (daemon, transport) = UnixStream::pair().unwrap();
            let (client, bridge) = UnixStream::pair().unwrap();
            mark_cloexec_nonblocking(transport.as_raw_fd()).unwrap();
            mark_cloexec_nonblocking(bridge.as_raw_fd()).unwrap();
            client.shutdown(std::net::Shutdown::Write).unwrap();
            let mut payload = mux_frame(1, b"one");
            payload.extend(mux_frame(2, b"diagnostic"));
            payload.extend(mux_frame(1, b"two"));
            let feeder = std::thread::spawn(move || {
                let mut daemon = daemon;
                daemon.write_all(&payload).unwrap();
            });
            let transport = Arc::new(transport);
            let bridge = Arc::new(bridge);
            let meter = SharedJournalBudget::default();
            pay(&meter, 256, 2).unwrap();
            let physical = Arc::new(PrepaidShutdown::bind(
                Arc::clone(&transport),
                Arc::new(std::sync::atomic::AtomicU8::new(0)),
            ));
            let output = Arc::new(PrepaidShutdown::bind(
                Arc::clone(&bridge),
                Arc::new(std::sync::atomic::AtomicU8::new(0)),
            ));
            let worker = std::thread::spawn(move || {
                mux(
                    transport,
                    bridge,
                    Instant::now() + Duration::from_secs(3),
                    meter,
                    physical,
                    output,
                    Arc::new(AtomicBool::new(false)),
                )
            });
            let mut client = client;
            let mut body = Vec::new();
            client.read_to_end(&mut body).unwrap();
            feeder.join().unwrap();
            let result = worker.join().unwrap().unwrap();
            assert_eq!(body, b"onetwo");
            assert!(result.double_eof && result.no_extra);
            assert_eq!(result.stdin_bytes, 0);
            assert_eq!(result.stdout, <[u8; 32]>::from(Sha256::digest(b"onetwo")));
            assert_eq!(
                result.stderr,
                <[u8; 32]>::from(Sha256::digest(b"diagnostic"))
            );
        }
        #[test]
        fn mux_rejects_truncated_headers_and_unknown_streams() {
            for raw in [
                vec![1, 0, 0, 0, 0],
                mux_frame(3, b"error"),
                vec![1, 1, 0, 0, 0, 0, 0, 1, b'x'],
            ] {
                let (mut daemon, transport) = UnixStream::pair().unwrap();
                let (client, bridge) = UnixStream::pair().unwrap();
                mark_cloexec_nonblocking(transport.as_raw_fd()).unwrap();
                mark_cloexec_nonblocking(bridge.as_raw_fd()).unwrap();
                client.shutdown(std::net::Shutdown::Write).unwrap();
                daemon.write_all(&raw).unwrap();
                drop(daemon);
                let transport = Arc::new(transport);
                let bridge = Arc::new(bridge);
                let meter = SharedJournalBudget::default();
                pay(&meter, 256, 2).unwrap();
                let physical = Arc::new(PrepaidShutdown::bind(
                    Arc::clone(&transport),
                    Arc::new(std::sync::atomic::AtomicU8::new(0)),
                ));
                let output = Arc::new(PrepaidShutdown::bind(
                    Arc::clone(&bridge),
                    Arc::new(std::sync::atomic::AtomicU8::new(0)),
                ));
                assert!(
                    mux(
                        transport,
                        bridge,
                        Instant::now() + Duration::from_secs(3),
                        meter,
                        physical,
                        output,
                        Arc::new(AtomicBool::new(false))
                    )
                    .is_err()
                );
            }
        }
    }
}

//! Private Root control DATA/native primitives and C11 durable DATA publication.
//!
//! This leaf is compiled as a private DATA component. No function
//! mints OriginalRootJob, a DB process, journal writer, session, cut or permit.
//! The eventual original Root job must own the fixed Catalog/companion mapping,
//! actual Docker before/after observations, child and control channel itself.
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
    sync::atomic::{AtomicBool, Ordering},
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
    meter.charge(
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
    file: File,
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
            file,
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
        let mut held = self.file.try_clone()?;
        let mut named = parent.open_file(leaf)?;
        for file in [&mut held, &mut named] {
            if Stamp::read(&file.metadata()?) != self.stamp {
                return Err(invalid());
            }
            file.seek(SeekFrom::Start(0))?;
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
    companion: BackupDir,
    journals: BackupDir,
    binding: BackupDir,
    epochs: BackupDir,
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
        companion: &BackupDir,
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
        let epochs = binding.open_dir("epochs")?;
        let epoch = epochs.open_dir(&notice.epoch_id.to_string())?;
        for directory in [&journals, &binding, &epochs, &epoch] {
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
            companion: companion.try_clone()?,
            journals,
            binding,
            epochs,
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
        for (parent, leaf, child) in [
            (&self.companion, "journals".to_owned(), &self.journals),
            (&self.journals, self.binding_id.to_string(), &self.binding),
            (&self.binding, "epochs".to_owned(), &self.epochs),
            (&self.epochs, self.epoch_id.to_string(), &self.epoch),
        ] {
            child.require_private_directory()?;
            if parent.open_dir(&leaf)?.identity()? != child.identity()? {
                return Err(invalid());
            }
        }
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
    if stop.load(Ordering::Acquire) || Instant::now() >= deadline {
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
        self.scan.charge(bytes.len() as u64, 1)?;
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

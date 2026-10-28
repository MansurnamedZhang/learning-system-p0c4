//! Private C11 DATA/held-genesis component, not an admitted Root observer.
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
    io::{Read, Seek, SeekFrom},
    os::unix::{fs::MetadataExt, io::AsRawFd},
};
use uuid::{Uuid, Variant, Version};

const RECORD_CAP: usize = 1024 * 1024;
const FRAME_CAP: usize = 4 * 1024 * 1024;
const CONTAINER_CAP: usize = 1024;
const ENTRY_CAP: usize = 100_000;
const GENESIS_NAME: &str = "00000000000000000001.frame";

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
        let mut file = parent.open_file(leaf)?;
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
        file.read_exact(&mut raw)?;
        let mut extra = [0];
        if file.read(&mut extra)? != 0 || Stamp::read(&file.metadata()?) != stamp {
            return Err(invalid());
        }
        pay(meter, count, 0)?;
        let digest = Sha256::digest(&raw).into();
        let held = Self {
            file,
            stamp,
            raw,
            digest,
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
                file.read_exact(&mut buffer[..count])?;
                if buffer[..count] != self.raw[offset..offset + count] {
                    return Err(invalid());
                }
                digest.update(&buffer[..count]);
                offset += count;
            }
            let mut extra = [0];
            let actual_digest: [u8; 32] = digest.finalize().into();
            if file.read(&mut extra)? != 0
                || actual_digest != self.digest
                || Stamp::read(&file.metadata()?) != self.stamp
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
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
}

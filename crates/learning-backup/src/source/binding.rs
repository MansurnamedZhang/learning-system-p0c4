//! Independent build-pinned identity of the source control root.
use crate::{BackupError, maintenance::valid_c4_database};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::{Uuid, Variant, Version};

const MAX_BINDING_BYTES: usize = 4096;

// A borrowed argument bundle only: no serde, constructor or authority owner.
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
pub(super) struct B1ClaimRefs<'a> {
    pub(super) project: &'a str,
    pub(super) container: &'a str,
    pub(super) image: &'a str,
    pub(super) image_digest: &'a str,
    pub(super) daemon: &'a str,
    pub(super) network: &'a str,
    pub(super) mounts: &'a str,
    pub(super) native: &'a super::endpoint::NativeIdentity,
}

#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
pub(crate) use b1::prepay_canonical_data;
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
pub(super) use b1::{B1InstallCutoverObservation, B1SourceInputs};
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
pub(crate) use b1::{B1PublicationInputs, B1PublicationPinView};

/// These are consumers of a currently held original session, never a second
/// qualification factory. The Root issuer's physical cutover observations
/// and the matching DB15 ACK owner remain separate integration obligations.
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
mod b1 {
    use super::*;
    use crate::registry::{EnrolledSource, RegistryLease, ScanBudget};
    use learning_assets::backup_fs::BackupDir;
    use learning_db::snapshot_broker::{B1InstalledSourceProcess, B1SourceIsolationInputData};
    use std::{
        fs::{File, Metadata},
        io::{Read, Seek, SeekFrom},
        os::unix::fs::MetadataExt,
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
    };

    type Budget = Arc<Mutex<ScanBudget>>;
    const OBSERVATION_ROOT: &str = "/var/lib/knowweave-c4/b1-source-observation";

    struct CanonicalHashWriter<'a> {
        budget: &'a Budget,
        hash: Sha256,
        bytes: usize,
        failure: Option<BackupError>,
    }
    impl std::io::Write for CanonicalHashWriter<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.failure.is_some() {
                return Err(std::io::Error::other("B1 canonical hash stopped"));
            }
            let observed = (|| {
                let next = self
                    .bytes
                    .checked_add(bytes.len())
                    .ok_or(BackupError::Overflow)?;
                if next > 16384 {
                    return Err(BackupError::Invalid("B1 canonical recipe size"));
                }
                self.budget
                    .lock()
                    .map_err(|_| BackupError::Invalid("B1 scan budget"))?
                    .charge((bytes.len() as u64) * 2)?;
                self.hash.update(bytes);
                self.bytes = next;
                Ok(())
            })();
            if let Err(error) = observed {
                self.failure = Some(error);
                return Err(std::io::Error::other("B1 canonical hash budget or size"));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    fn hash_canonical_value(
        value: &serde_json::Value,
        budget: &Budget,
    ) -> Result<String, BackupError> {
        // This Value came from the exact canonical Catalog. Stream the same
        // canonical Value bytes, paying each chunk before serialization/hash;
        // no full output Vec or second Value clone is allocated.
        budget
            .lock()
            .map_err(|_| BackupError::Invalid("B1 scan budget"))?
            .entries(1)?;
        let mut writer = CanonicalHashWriter {
            budget,
            hash: Sha256::new(),
            bytes: 0,
            failure: None,
        };
        let encoded = serde_json::to_writer(&mut writer, value);
        if let Some(error) = writer.failure {
            return Err(error);
        }
        encoded?;
        budget
            .lock()
            .map_err(|_| BackupError::Invalid("B1 scan budget"))?
            .charge(64)?;
        Ok(hex::encode(writer.hash.finalize()))
    }
    pub(crate) fn prepay_canonical_data(raw: &[u8], budget: &Budget) -> Result<(), BackupError> {
        // Fixed stack/no heap before typed parsing. Count actual container,
        // field and separator work outside strings conservatively; malformed
        // or excessive nesting is rejected before serde can allocate arrays.
        budget
            .lock()
            .map_err(|_| BackupError::Invalid("B1 scan budget"))?
            .charge(raw.len() as u64)?;
        let (mut quoted, mut escaped, mut depth, mut entries) = (false, false, 0usize, 0usize);
        for &byte in raw {
            if quoted {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    quoted = false;
                }
                continue;
            }
            match byte {
                b'"' => quoted = true,
                b'[' | b'{' => {
                    depth = depth.checked_add(1).ok_or(BackupError::Overflow)?;
                    if depth > 128 {
                        return Err(BackupError::Invalid("B1 bounded JSON nesting"));
                    }
                    entries = entries.checked_add(1).ok_or(BackupError::Overflow)?;
                }
                b']' | b'}' => {
                    depth = depth
                        .checked_sub(1)
                        .ok_or(BackupError::Invalid("B1 JSON nesting"))?;
                }
                b',' | b':' => {
                    entries = entries.checked_add(1).ok_or(BackupError::Overflow)?;
                }
                _ => {}
            }
        }
        if quoted || depth != 0 {
            return Err(BackupError::Invalid("B1 JSON framing"));
        }
        let mut scan = budget
            .lock()
            .map_err(|_| BackupError::Invalid("B1 scan budget"))?;
        scan.entries(entries)?;
        // Reserve typed serde + Value/canonical copies before their first
        // allocation, including rejected duplicate/unknown input. This is the
        // same cumulative backing, never a new per-record allowance. A body
        // below its shape cap may still legitimately exhaust the old scope.
        scan.charge(
            (raw.len() as u64)
                .checked_mul(64)
                .ok_or(BackupError::Overflow)?,
        )?;
        Ok(())
    }

    #[derive(PartialEq, Eq)]
    struct Stamp {
        dev: u64,
        ino: u64,
        mode: u32,
        uid: u32,
        gid: u32,
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
                mode: m.mode(),
                uid: m.uid(),
                gid: m.gid(),
                links: m.nlink(),
                bytes: m.len(),
                mtime: m.mtime(),
                mtime_ns: m.mtime_nsec(),
                ctime: m.ctime(),
                ctime_ns: m.ctime_nsec(),
            }
        }
    }
    struct HeldRecord {
        root: BackupDir,
        parent: PathBuf,
        parent_identity: (u64, u64),
        name: String,
        file: File,
        stamp: Stamp,
        raw: Vec<u8>,
        digest: [u8; 32],
    }
    fn read_held_bytes(
        reader: &mut impl Read,
        size: u64,
        budget: &Budget,
    ) -> Result<Vec<u8>, BackupError> {
        // The common reader already prepays the requested IO. Reserve this
        // B1 caller's independent Vec allocation before entering that reader.
        budget
            .lock()
            .map_err(|_| BackupError::Invalid("B1 scan budget"))?
            .charge(size)?;
        crate::registry::read_metadata(reader, size, budget)
    }
    impl HeldRecord {
        fn open(
            parent: &Path,
            name: String,
            cap: u64,
            budget: &Budget,
        ) -> Result<Self, BackupError> {
            if name.is_empty() || name.contains('/') || name.contains('\0') {
                return Err(BackupError::Invalid("B1 fixed metadata leaf"));
            }
            // Ancestor metadata is charged before the first open or stat.
            {
                let mut scan = budget
                    .lock()
                    .map_err(|_| BackupError::Invalid("B1 scan budget"))?;
                scan.entries(parent.components().count().saturating_mul(4) + 4)?;
                scan.charge(4096)?;
            }
            let root = BackupDir::open_trusted_private_root(parent)?;
            let parent_identity = root.identity()?;
            let mut file = root.open_file(&name)?;
            let meta = file.metadata()?;
            if !meta.is_file()
                || meta.uid() != 0
                || meta.gid() != 0
                || meta.mode() & 0o7777 != 0o600
                || meta.nlink() != 1
                || meta.len() == 0
                || meta.len() > cap
            {
                return Err(BackupError::Invalid("B1 private metadata file"));
            }
            let stamp = Stamp::read(&meta);
            let raw = read_held_bytes(&mut file, meta.len(), budget)?;
            budget
                .lock()
                .map_err(|_| BackupError::Invalid("B1 scan budget"))?
                .charge(meta.len())?;
            let digest = Sha256::digest(&raw).into();
            let held = Self {
                root,
                parent: parent.to_owned(),
                parent_identity,
                name,
                file,
                stamp,
                raw,
                digest,
            };
            held.recheck(budget)?;
            Ok(held)
        }
        fn recheck(&self, budget: &Budget) -> Result<(), BackupError> {
            {
                let mut scan = budget
                    .lock()
                    .map_err(|_| BackupError::Invalid("B1 scan budget"))?;
                // Reserve ancestor reopen, held/named metadata before/after,
                // and the descriptor clone before those observations occur.
                scan.entries(self.parent.components().count().saturating_mul(4) + 10)?;
                scan.charge(4096)?;
            }
            self.root.require_private_directory()?;
            if self.root.identity()? != self.parent_identity
                || BackupDir::open_trusted_private_root(&self.parent)?.identity()?
                    != self.parent_identity
            {
                return Err(BackupError::Invalid("B1 metadata parent changed"));
            }
            if Stamp::read(&self.file.metadata()?) != self.stamp {
                return Err(BackupError::Invalid("B1 held metadata changed"));
            }
            let mut named = self.root.open_file(&self.name)?;
            if Stamp::read(&named.metadata()?) != self.stamp {
                return Err(BackupError::Invalid("B1 named metadata changed"));
            }
            let raw = read_held_bytes(&mut named, self.stamp.bytes, budget)?;
            budget
                .lock()
                .map_err(|_| BackupError::Invalid("B1 scan budget"))?
                .charge(self.stamp.bytes)?;
            if raw != self.raw || Stamp::read(&named.metadata()?) != self.stamp {
                return Err(BackupError::Invalid("B1 metadata bytes changed"));
            }
            let mut held = self.file.try_clone()?;
            held.seek(SeekFrom::Start(0))?;
            let held_raw = read_held_bytes(&mut held, self.stamp.bytes, budget)?;
            let mut extra = [0];
            // Allocation/read were prepaid separately. Pay this full byte
            // comparison and the EOF probe even when it returns zero bytes.
            {
                let mut scan = budget
                    .lock()
                    .map_err(|_| BackupError::Invalid("B1 scan budget"))?;
                scan.charge(
                    self.stamp
                        .bytes
                        .checked_add(1)
                        .ok_or(BackupError::Overflow)?,
                )?;
                scan.entries(1)?;
            }
            if held_raw != self.raw
                || held.read(&mut extra)? != 0
                || Stamp::read(&held.metadata()?) != self.stamp
            {
                return Err(BackupError::Invalid("B1 original metadata bytes changed"));
            }
            Ok(())
        }
        fn digest(&self) -> [u8; 32] {
            self.digest
        }
    }

    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct InstalledSource {
        deployment_id: String,
        source_group_id: String,
        application_commit: String,
        application_build_sha256: String,
        source_auth_public_key_sha256: String,
        registry_authority_sha256: String,
        source_group_record_sha256: String,
        source_control_binding_sha256: String,
        database: String,
        database_oid: u64,
        system_identifier: String,
    }
    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct InstalledRuntime {
        compose_project: String,
        pg_container_id: String,
        pg_image_ref: String,
        daemon_id: String,
        internal_network_id: String,
        capture_mount_recipe_sha256: String,
        source_root_records_sha256: String,
        pg_pid_namespace: String,
        pg_mount_namespace: String,
        postmaster_start_ticks: u64,
        admitted_pg_image_id: String,
        pg_image_digest: String,
    }
    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Catalog {
        schema_version: u64,
        capability: String,
        profile: String,
        launch_id: String,
        installed_source: InstalledSource,
        peer_endpoint: serde_json::Value,
        capture_recipe_sha256: String,
        images: serde_json::Value,
        installed_runtime: InstalledRuntime,
        capture_exec_recipe: serde_json::Value,
    }
    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Generation {
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
    #[derive(Serialize, Deserialize)]
    #[serde(untagged)]
    enum NullableString {
        Text(String),
        Null(()),
    }
    fn present_nullable<'de, D: serde::Deserializer<'de>>(
        d: D,
    ) -> Result<Option<NullableString>, D::Error> {
        NullableString::deserialize(d).map(Some)
    }
    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields, rename_all = "PascalCase")]
    struct ActualAssetsMount {
        #[serde(rename = "Type")]
        kind: String,
        source: String,
        destination: String,
        #[serde(rename = "RW")]
        rw: bool,
        #[serde(
            default,
            deserialize_with = "present_nullable",
            skip_serializing_if = "Option::is_none"
        )]
        name: Option<NullableString>,
        #[serde(
            default,
            deserialize_with = "present_nullable",
            skip_serializing_if = "Option::is_none"
        )]
        driver: Option<NullableString>,
        #[serde(
            default,
            deserialize_with = "present_nullable",
            skip_serializing_if = "Option::is_none"
        )]
        mode: Option<NullableString>,
        #[serde(
            default,
            deserialize_with = "present_nullable",
            skip_serializing_if = "Option::is_none"
        )]
        propagation: Option<NullableString>,
    }
    fn required_nullable<'de, D: serde::Deserializer<'de>>(
        d: D,
    ) -> Result<Option<String>, D::Error> {
        Option::<String>::deserialize(d)
    }
    #[derive(Serialize, Deserialize, PartialEq, Eq)]
    #[serde(deny_unknown_fields)]
    struct MountProjection {
        destination: String,
        source: String,
        kind: String,
        rw: bool,
        #[serde(deserialize_with = "required_nullable")]
        name: Option<String>,
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
        #[serde(deserialize_with = "required_nullable")]
        project: Option<String>,
        #[serde(deserialize_with = "required_nullable")]
        service: Option<String>,
        mounts: Vec<MountProjection>,
    }
    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct HostCutover {
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
    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct InstallObservation {
        schema_version: u64,
        capability: String,
        job_id: String,
        backup_id: String,
        phase: String,
        request_id: String,
        binding_id: Uuid,
        epoch_id: Uuid,
        generation_binding_sha256: String,
        descriptor_sha256: String,
        epoch_header_sha256: String,
        genesis_frame_sha256: String,
        observed_unix_ms: u64,
        host_cutover: HostCutover,
    }
    pub(in crate::source) struct B1InstallCutoverObservation {
        record: HeldRecord,
        header: HeldRecord,
        genesis: HeldRecord,
        journal: BackupDir,
        journal_path: PathBuf,
        journal_identity: (u64, u64),
        broker_lock: File,
        broker_lock_stamp: Stamp,
        data: InstallObservation,
    }
    impl B1InstallCutoverObservation {
        pub(in crate::source) fn recheck(&self, budget: &Budget) -> Result<(), BackupError> {
            self.record.recheck(budget)?;
            self.header.recheck(budget)?;
            self.genesis.recheck(budget)?;
            budget
                .lock()
                .map_err(|_| BackupError::Invalid("B1 scan budget"))?
                .entries(self.journal_path.components().count() * 4 + 8)?;
            if self.journal.identity()? != self.journal_identity
                || BackupDir::open_trusted_private_root(&self.journal_path)?.identity()?
                    != self.journal_identity
                || Stamp::read(&self.broker_lock.metadata()?) != self.broker_lock_stamp
                || Stamp::read(&self.journal.open_file("broker.lock")?.metadata()?)
                    != self.broker_lock_stamp
            {
                return Err(BackupError::Invalid(
                    "B1 original install journal or broker lock changed",
                ));
            }
            use std::os::fd::AsRawFd;
            if unsafe { libc::flock(self.broker_lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }
                == 0
            {
                unsafe { libc::flock(self.broker_lock.as_raw_fd(), libc::LOCK_UN) };
                return Err(BackupError::Invalid("B1 original broker live lock lost"));
            }
            if std::io::Error::last_os_error().raw_os_error() != Some(libc::EWOULDBLOCK) {
                return Err(BackupError::Invalid("B1 original broker lock unconfirmed"));
            }
            Ok(())
        }
        pub(in crate::source) fn host_data(
            &self,
            budget: &Budget,
        ) -> Result<serde_json::Value, BackupError> {
            self.recheck(budget)?;
            prepay_canonical_data(&self.record.raw, budget)?;
            Ok(serde_json::to_value(&self.data.host_cutover)?)
        }
    }
    pub(in crate::source) struct B1SourceInputs {
        catalog_file: HeldRecord,
        catalog: Catalog,
        generation_file: HeldRecord,
        generation: Generation,
        capture_start: HeldRecord,
        proof: HeldRecord,
        inspection: HeldRecord,
        generation_lock: File,
        generation_lock_stamp: Stamp,
        job_id: Uuid,
        backup_id: Uuid,
        phase: &'static str,
    }
    impl B1SourceInputs {
        pub(in crate::source) fn read(
            peer: &Arc<B1InstalledSourceProcess>,
            data: &B1SourceIsolationInputData,
            budget: &Budget,
        ) -> Result<Self, BackupError> {
            let valid_id = |id: Uuid| {
                id.get_version() == Some(Version::Random) && id.get_variant() == Variant::RFC4122
            };
            if !valid_id(data.job_id()) || !valid_id(data.backup_id()) {
                return Err(BackupError::Invalid("B1 current generation UUID"));
            }
            let role_catalog = match option_env!("KNOWWEAVE_C4_B1_SOURCE_PROCESS_ROLE") {
                Some("publication-source") => "/var/lib/knowweave-c4/b1",
                Some("capture") => "/opt/knowweave-c4/b1-capture",
                _ => return Err(BackupError::Invalid("B1 compiled process role")),
            };
            peer.revalidate()
                .map_err(|_| BackupError::Invalid("B1 source before Catalog"))?;
            let catalog_file = HeldRecord::open(
                Path::new(role_catalog),
                "source-peer-execution.json".into(),
                16384,
                budget,
            )?;
            if catalog_file.digest() != data.catalog_sha256() {
                return Err(BackupError::Invalid("B1 same-session Catalog digest"));
            }
            prepay_canonical_data(&catalog_file.raw, budget)?;
            let catalog: Catalog = crate::registry::canonical_record(&catalog_file.raw, 16384)?;
            if catalog.schema_version != 1
                || catalog.capability != "b1_source_peer_execution_v1"
                || catalog.profile != "task9-b1-limited-v1"
                || !crate::registry::v4(&catalog.launch_id)
                || !valid_c4_database(&catalog.installed_source.database)
            {
                return Err(BackupError::Invalid("B1 installed Catalog fields"));
            }
            let phase_directory = match data.phase() {
                "Install" => "install",
                "Capture" => "capture",
                _ => return Err(BackupError::Invalid("B1 current generation phase")),
            };
            let backup_root = PathBuf::from(format!(
                "{OBSERVATION_ROOT}/jobs/{}/backups/{}",
                data.job_id(),
                data.backup_id()
            ));
            let generation_root = backup_root.join(phase_directory);
            let generation_file = HeldRecord::open(
                &generation_root,
                "generation-binding.json".into(),
                4096,
                budget,
            )?;
            if generation_file.digest() != data.generation_binding_sha256() {
                return Err(BackupError::Invalid("B1 same-session generation digest"));
            }
            prepay_canonical_data(&generation_file.raw, budget)?;
            let generation: Generation =
                crate::registry::canonical_record(&generation_file.raw, 4096)?;
            if generation.schema_version != 1
                || generation.capability != "b1_immutable_isolation_generation_v1"
                || generation.job_id != data.job_id().to_string()
                || generation.backup_id != data.backup_id().to_string()
                || generation.phase != data.phase()
                || !crate::registry::v4(&generation.request_id)
                || !crate::valid_digest(&generation.capture_start_sha256)
                || generation.isolation_bytes == 0
                || generation.isolation_bytes > 4096
                || generation.inspection_bytes == 0
                || generation.inspection_bytes > 1024 * 1024
                || !crate::valid_digest(&generation.isolation_sha256)
                || !crate::valid_digest(&generation.inspection_sha256)
                || generation.lock_dev == 0
                || generation.lock_ino == 0
                || generation.observed_unix_ms == 0
            {
                return Err(BackupError::Invalid("B1 current generation fields"));
            }
            let capture_start =
                HeldRecord::open(&backup_root, "capture-start.json".into(), 16384, budget)?;
            if hex::encode(capture_start.digest()) != generation.capture_start_sha256 {
                return Err(BackupError::Invalid("B1 original capture-start digest"));
            }
            // The same ctx has validated the full exact CaptureStart schema.
            // This local DATA projection additionally joins consumed fields.
            prepay_canonical_data(&capture_start.raw, budget)?;
            let capture: serde_json::Value =
                crate::registry::canonical_record(&capture_start.raw, 16384)?;
            if capture["job_id"] != generation.job_id
                || capture["backup_id"] != generation.backup_id
                || capture["catalog_sha256"] != hex::encode(data.catalog_sha256())
                || capture["mount_recipe_sha256"] != catalog.capture_recipe_sha256
                || capture["mount_recipe_sha256"]
                    != catalog.installed_runtime.capture_mount_recipe_sha256
            {
                return Err(BackupError::Invalid("B1 current capture-start fields"));
            }
            // Hexec is a distinct preimage from the actual twelve mount rows.
            if capture["capture_exec_recipe_sha256"]
                != hash_canonical_value(&catalog.capture_exec_recipe, budget)?
            {
                return Err(BackupError::Invalid("B1 current capture exec recipe"));
            }
            let proof = HeldRecord::open(
                &generation_root,
                format!("isolation-{}.json", data.backup_id()),
                4096,
                budget,
            )?;
            let inspection = HeldRecord::open(
                &generation_root,
                format!("inspection-{}.json", data.backup_id()),
                1024 * 1024,
                budget,
            )?;
            if proof.raw.len() as u64 != generation.isolation_bytes
                || hex::encode(proof.digest()) != generation.isolation_sha256
                || inspection.raw.len() as u64 != generation.inspection_bytes
                || hex::encode(inspection.digest()) != generation.inspection_sha256
            {
                return Err(BackupError::Invalid(
                    "B1 generation original proof and inspection",
                ));
            }
            budget
                .lock()
                .map_err(|_| BackupError::Invalid("B1 scan budget"))?
                .entries(2)?;
            let generation_lock = generation_file
                .root
                .open_file(&format!("isolation-{}.lock", data.backup_id()))?;
            let lock_meta = generation_lock.metadata()?;
            if !lock_meta.is_file()
                || lock_meta.uid() != 0
                || lock_meta.gid() != 0
                || lock_meta.mode() & 0o7777 != 0o600
                || lock_meta.nlink() != 1
                || lock_meta.len() != 0
                || (lock_meta.dev(), lock_meta.ino()) != (generation.lock_dev, generation.lock_ino)
            {
                return Err(BackupError::Invalid("B1 current original generation lock"));
            }
            let generation_lock_stamp = Stamp::read(&lock_meta);
            let inputs = Self {
                catalog_file,
                catalog,
                generation_file,
                generation,
                capture_start,
                proof,
                inspection,
                generation_lock,
                generation_lock_stamp,
                job_id: data.job_id(),
                backup_id: data.backup_id(),
                phase: data.phase(),
            };
            inputs.recheck(peer, data, budget)?;
            Ok(inputs)
        }
        pub(in crate::source) fn database(&self) -> &str {
            &self.catalog.installed_source.database
        }
        pub(in crate::source) fn phase(&self) -> &'static str {
            self.phase
        }
        /// DATA projected only from this original Capture issuance. Keeping
        /// these held records after its actual SQL close does not reopen SQL.
        pub(in crate::source) fn capture_identity_data(
            &self,
        ) -> Result<(Uuid, Uuid, [u8; 32], [u8; 32]), BackupError> {
            if self.phase != "Capture" {
                return Err(BackupError::Invalid("B1 original Capture inputs required"));
            }
            Ok((
                self.job_id,
                self.backup_id,
                self.capture_start.digest(),
                self.generation_file.digest(),
            ))
        }
        pub(in crate::source) fn install_observation(
            &self,
            descriptor: &[u8],
            epoch: Uuid,
            binding: Uuid,
            budget: &Budget,
        ) -> Result<B1InstallCutoverObservation, BackupError> {
            if self.phase != "Install" {
                return Err(BackupError::Invalid(
                    "B1 original Install generation required",
                ));
            }
            let record = HeldRecord::open(
                &self.generation_file.parent,
                "install-cutover-observation.json".into(),
                1024 * 1024,
                budget,
            )?;
            // Same current bounded bytes, not a caller DTO. This exact shape
            // is the human-approved C11 draft3 record; unknowns reject.
            prepay_canonical_data(&record.raw, budget)?;
            let data: InstallObservation =
                crate::registry::canonical_record(&record.raw, 1024 * 1024)?;
            budget
                .lock()
                .map_err(|_| BackupError::Invalid("B1 scan budget"))?
                .charge((descriptor.len() as u64) * 2 + 4 * 64)?;
            if data.schema_version != 1
                || data.capability != "b1_install_cutover_observation_v1"
                || data.job_id != self.job_id.to_string()
                || data.backup_id != self.backup_id.to_string()
                || data.phase != "Install"
                || data.request_id != self.generation.request_id
                || data.binding_id != binding
                || data.epoch_id != epoch
                || data.generation_binding_sha256 != hex::encode(self.generation_file.digest())
                || data.descriptor_sha256 != crate::digest(descriptor)
                || data.observed_unix_ms == 0
                || !crate::valid_digest(&data.epoch_header_sha256)
                || !crate::valid_digest(&data.genesis_frame_sha256)
            {
                return Err(BackupError::Invalid(
                    "B1 original install observation association",
                ));
            }
            let parent = PathBuf::from(format!(
                "/var/lib/knowweave-c4/snapshot-effects-v2/journals/{binding}/epochs/{epoch}"
            ));
            let header = HeldRecord::open(&parent, "epoch.header".into(), 40, budget)?;
            let genesis = HeldRecord::open(
                &parent,
                "00000000000000000001.frame".into(),
                4 * 1024 * 1024,
                budget,
            )?;
            let h = &header.raw;
            let f = &genesis.raw;
            budget
                .lock()
                .map_err(|_| BackupError::Invalid("B1 scan budget"))?
                .charge((f.len() as u64) + 4 * 64)?;
            if h.len() != 40
                || &h[..8] != b"KWSJEPO2"
                || &h[8..24] != epoch.as_bytes()
                || &h[24..40] != binding.as_bytes()
                || f.len() != 120 + descriptor.len()
                || &f[..8] != b"KWSJBYT2"
                || f[8..16] != 1u64.to_be_bytes()
                || &f[16..32] != epoch.as_bytes()
                || &f[32..48] != binding.as_bytes()
                || f[48..80] != header.digest()
                || f[80..112] != Sha256::digest(descriptor)[..]
                || f[112..120] != (descriptor.len() as u64).to_be_bytes()
                || &f[120..] != descriptor
                || hex::encode(header.digest()) != data.epoch_header_sha256
                || hex::encode(genesis.digest()) != data.genesis_frame_sha256
            {
                return Err(BackupError::Invalid(
                    "B1 exact current install genesis DATA",
                ));
            }
            let names = crate::registry::prepaid_b1_roster(&header.root, budget, 2)?;
            if names != ["00000000000000000001.frame", "epoch.header"] {
                return Err(BackupError::Invalid(
                    "B1 install genesis exact original roster",
                ));
            }
            self.require_host_cutover(&data.host_cutover, budget)?;
            // Descriptor's journal identity denotes the binding broker
            // directory, not the nested epoch directory. Connect the exact
            // bytes to that actual current directory and its live owner lock.
            prepay_canonical_data(descriptor, budget)?;
            let actual_descriptor: super::super::snapshot_effects::wire::Descriptor =
                serde_json::from_slice(descriptor)?;
            let journal_path = PathBuf::from(format!(
                "/var/lib/knowweave-c4/snapshot-effects-v2/journals/{binding}"
            ));
            budget
                .lock()
                .map_err(|_| BackupError::Invalid("B1 scan budget"))?
                .entries(journal_path.components().count() * 4 + 8)?;
            let journal = BackupDir::open_trusted_private_root(&journal_path)?;
            let journal_identity = journal.identity()?;
            if journal_identity
                != (
                    actual_descriptor.journal_root_dev,
                    actual_descriptor.journal_root_ino,
                )
            {
                return Err(BackupError::Invalid(
                    "B1 descriptor original journal binding directory",
                ));
            }
            let broker_lock = journal.open_file("broker.lock")?;
            let metadata = broker_lock.metadata()?;
            if !metadata.is_file()
                || metadata.uid() != 0
                || metadata.gid() != 0
                || metadata.mode() & 0o7777 != 0o600
                || metadata.nlink() != 1
                || metadata.len() != 0
            {
                return Err(BackupError::Invalid("B1 original broker lock metadata"));
            }
            let broker_lock_stamp = Stamp::read(&metadata);
            let observed = B1InstallCutoverObservation {
                record,
                header,
                genesis,
                journal,
                journal_path,
                journal_identity,
                broker_lock,
                broker_lock_stamp,
                data,
            };
            observed.recheck(budget)?;
            Ok(observed)
        }
        fn require_host_cutover(
            &self,
            host: &HostCutover,
            budget: &Budget,
        ) -> Result<(), BackupError> {
            let runtime = &self.catalog.installed_runtime;
            let path = |s: &str| s.len() <= 4096 && crate::registry::canonical_path(s);
            let string = |s: &str, cap| s.len() <= cap && !s.contains('\0');
            let nullable = |v: &Option<NullableString>, cap| {
                v.as_ref().is_none_or(|v| match v {
                    NullableString::Text(s) => string(s, cap),
                    NullableString::Null(()) => true,
                })
            };
            if host.container_id != runtime.pg_container_id
                || host.image_sha256 != runtime.admitted_pg_image_id
                || host.daemon_id != runtime.daemon_id
                || !path(&host.export_mount)
                || !path(&host.journal_mount)
                || host.export_mount == host.journal_mount
                || host
                    .export_mount
                    .strip_prefix(&host.journal_mount)
                    .is_some_and(|s| s.starts_with('/'))
                || host
                    .journal_mount
                    .strip_prefix(&host.export_mount)
                    .is_some_and(|s| s.starts_with('/'))
                || !crate::valid_digest(&host.mountinfo_sha256)
                || option_env!("KNOWWEAVE_C4_DOCKER_CLI_SHA256")
                    != Some(host.docker_cli_sha256.as_str())
                || !crate::valid_digest(&host.docker_cli_sha256)
                || host.assets_mount.rw
                || !["bind", "volume"].contains(&host.assets_mount.kind.as_str())
                || !path(&host.assets_mount.source)
                || !path(&host.assets_mount.destination)
                || !nullable(&host.assets_mount.name, 4096)
                || !nullable(&host.assets_mount.driver, 128)
                || !nullable(&host.assets_mount.mode, 256)
                || !nullable(&host.assets_mount.propagation, 128)
                || host.complete_container_projection.is_empty()
                || host.complete_container_projection.len() > 1024
                || host.legacy_producers.len() > 1024
            {
                return Err(BackupError::Invalid(
                    "B1 complete original host cutover DATA",
                ));
            }
            let count = host
                .complete_container_projection
                .iter()
                .try_fold(host.complete_container_projection.len(), |n, c| {
                    n.checked_add(c.mounts.len()).ok_or(BackupError::Overflow)
                })?;
            budget
                .lock()
                .map_err(|_| BackupError::Invalid("B1 scan budget"))?
                .entries(count)?;
            let mut last = "";
            for row in &host.complete_container_projection {
                if !crate::valid_digest(&row.container_id)
                    || row.container_id.as_str() <= last
                    || !row
                        .image_sha256
                        .strip_prefix("sha256:")
                        .is_some_and(crate::valid_digest)
                    || !string(&row.started_at, 128)
                    || row.project.as_ref().is_some_and(|v| !string(v, 1024))
                    || row.service.as_ref().is_some_and(|v| !string(v, 1024))
                {
                    return Err(BackupError::Invalid(
                        "B1 complete original container projection",
                    ));
                }
                last = &row.container_id;
                let mut destination = "";
                for mount in &row.mounts {
                    if !path(&mount.source)
                        || !path(&mount.destination)
                        || mount.destination.as_str() <= destination
                        || !string(&mount.kind, 64)
                        || mount.name.as_ref().is_some_and(|v| !string(v, 4096))
                    {
                        return Err(BackupError::Invalid(
                            "B1 complete original mount projection",
                        ));
                    }
                    destination = &mount.destination;
                }
            }
            let legacy = host
                .complete_container_projection
                .iter()
                .filter(|r| {
                    r.project.as_deref() == Some(runtime.compose_project.as_str())
                        && r.container_id != host.container_id
                })
                .collect::<Vec<_>>();
            if legacy.len() != host.legacy_producers.len()
                || !legacy
                    .iter()
                    .zip(&host.legacy_producers)
                    .all(|(a, b)| *a == b)
            {
                return Err(BackupError::Invalid(
                    "B1 exact original legacy producer projection",
                ));
            }
            let current = host
                .complete_container_projection
                .iter()
                .find(|r| r.container_id == host.container_id)
                .ok_or(BackupError::Invalid(
                    "B1 installed PG missing from full host set",
                ))?;
            if !current.running
                || current.image_sha256 != host.image_sha256
                || current.project.as_deref() != Some(runtime.compose_project.as_str())
                || current.service.as_deref() != Some("pg")
                || current.mounts.len() != 12
            {
                return Err(BackupError::Invalid("B1 exact current PG host projection"));
            }
            let topology = [
                ("/opt/knowweave-c4/b1-capture", "bind", false),
                ("/run/kwc4/sfx", "bind", true),
                ("/usr/bin/docker", "bind", false),
                ("/var/lib/knowweave-c4/b1-source-observation", "bind", false),
                ("/var/lib/knowweave-c4/registry", "volume", true),
                ("/var/lib/knowweave-c4/snapshot-effects-v2", "volume", true),
                ("/var/lib/knowweave-c4/snapshot-export-v2", "volume", true),
                ("/var/lib/knowweave-source", "volume", true),
                ("/var/lib/knowweave-source/assets", "bind", false),
                ("/var/lib/postgresql", "volume", true),
                ("/var/run/docker.sock", "bind", false),
                ("/var/run/postgresql", "volume", true),
            ];
            for (mount, (destination, kind, rw)) in current.mounts.iter().zip(topology) {
                if mount.destination != destination
                    || mount.kind != kind
                    || mount.rw != rw
                    || (kind == "volume" && mount.name.as_ref().is_none_or(|n| n.is_empty()))
                {
                    return Err(BackupError::Invalid("B1 exact twelve host mount DATA"));
                }
            }
            // This rehash connects this exact current full observation to the
            // installed twelve-row preimage; it is not physical observation.
            let work = current.mounts.iter().try_fold(2u64, |n, r| {
                let strings = r
                    .source
                    .len()
                    .checked_add(r.destination.len())
                    .and_then(|n| n.checked_add(r.kind.len()))
                    .and_then(|n| n.checked_add(r.name.as_ref().map_or(0, String::len)))
                    .ok_or(BackupError::Overflow)?;
                let escaped = (strings as u64)
                    .checked_mul(6)
                    .and_then(|n| n.checked_add(256))
                    .ok_or(BackupError::Overflow)?;
                n.checked_add(escaped).ok_or(BackupError::Overflow)
            })?;
            {
                let mut scan = budget
                    .lock()
                    .map_err(|_| BackupError::Invalid("B1 scan budget"))?;
                scan.charge(work.checked_mul(6).ok_or(BackupError::Overflow)?)?;
                scan.entries(current.mounts.len() * 8)?;
            }
            let mounts = current
                .mounts
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "destination":r.destination,"source":r.source,"kind":r.kind,"rw":r.rw,
                        "name":r.name.as_deref().unwrap_or("")
                    })
                })
                .collect::<Vec<_>>();
            let raw = crate::registry::canonical(&mounts)?;
            if crate::digest(&raw) != runtime.capture_mount_recipe_sha256
                || crate::digest(&raw) != self.catalog.capture_recipe_sha256
            {
                return Err(BackupError::Invalid(
                    "B1 actual twelve-row mount digest domain",
                ));
            }
            let companion = current
                .mounts
                .iter()
                .find(|r| r.destination == "/var/lib/knowweave-c4/snapshot-effects-v2")
                .expect("exact twelve rows");
            let export = current
                .mounts
                .iter()
                .find(|r| r.destination == "/var/lib/knowweave-c4/snapshot-export-v2")
                .expect("exact twelve rows");
            let assets = current
                .mounts
                .iter()
                .find(|r| r.destination == "/var/lib/knowweave-source/assets")
                .expect("exact twelve rows");
            if companion.source != host.journal_mount
                || export.source != host.export_mount
                || companion.name == export.name
                || assets.rw
                || host.assets_mount.source != assets.source
                || host.assets_mount.destination != assets.destination
                || host.assets_mount.kind != assets.kind
                || !(match (&host.assets_mount.name, &assets.name) {
                    (Some(NullableString::Text(a)), Some(b)) => a == b,
                    (None | Some(NullableString::Null(())), None) => true,
                    _ => false,
                })
            {
                return Err(BackupError::Invalid("B1 host mapped original source roots"));
            }
            for row in &host.complete_container_projection {
                if row.container_id == host.container_id {
                    continue;
                }
                if row.running
                    && row.project.as_deref() == Some(runtime.compose_project.as_str())
                    && row.service.as_deref() != Some("pg")
                {
                    return Err(BackupError::Invalid(
                        "B1 host DATA has a running old producer",
                    ));
                }
                for mount in &row.mounts {
                    let overlaps = |root: &str| {
                        mount.source == root
                            || mount
                                .source
                                .strip_prefix(root)
                                .is_some_and(|s| s.starts_with('/'))
                            || root
                                .strip_prefix(&mount.source)
                                .is_some_and(|s| s.starts_with('/'))
                    };
                    if mount.rw && (overlaps(&host.export_mount) || overlaps(&host.journal_mount)) {
                        return Err(BackupError::Invalid(
                            "B1 host DATA has a RW export or journal alias",
                        ));
                    }
                }
            }
            // M4's actual before/after all-state observations remain issuer
            // responsibilities; this typed DATA check does not claim them.
            Ok(())
        }
        pub(in crate::source) fn proof_path(&self) -> PathBuf {
            self.proof.parent.join(&self.proof.name)
        }
        pub(in crate::source) fn configuration(
            &self,
            enrolled: &EnrolledSource,
        ) -> Result<super::super::SourceBackupConfig, BackupError> {
            if enrolled.group.database != self.database()
                || enrolled.group.group_id != self.catalog.installed_source.source_group_id
                || enrolled.group.deployment_id != self.catalog.installed_source.deployment_id
            {
                return Err(BackupError::Invalid(
                    "B1 original source configuration group",
                ));
            }
            let control = &enrolled.roots["control"];
            let pins = &enrolled.roots["local_pins"];
            if !crate::registry::canonical_path(&control.path)
                || !crate::registry::canonical_path(&pins.path)
                || enrolled.handles["control"].identity()? != (control.dev, control.ino)
                || enrolled.handles["local_pins"].identity()? != (pins.dev, pins.ino)
            {
                return Err(BackupError::Invalid(
                    "B1 original source configuration roots",
                ));
            }
            Ok(super::super::SourceBackupConfig {
                backup_id: self.backup_id,
                expected_database: self.database().to_owned(),
                expected_compose_project: self.catalog.installed_runtime.compose_project.clone(),
                isolation_attestation: self.proof_path(),
                control_root: PathBuf::from(&control.path),
                local_pin_root: PathBuf::from(&pins.path),
                pg_dump_executable: PathBuf::from("/usr/lib/postgresql/18/bin/pg_dump"),
                pgpassfile: PathBuf::from("/dev/null"),
                pg_host: "/var/run/postgresql".into(),
                pg_port: 5432,
                drain_timeout: std::time::Duration::from_secs(60),
            })
        }
        pub(in crate::source) fn read_control_root(
            &self,
            path: &Path,
            budget: &Budget,
        ) -> Result<(BackupDir, Vec<u8>), BackupError> {
            let held = HeldRecord::open(
                path,
                "source-binding.json".into(),
                MAX_BINDING_BYTES as u64,
                budget,
            )?;
            let record = parse_pinned(
                &held.raw,
                option_env!("KNOWWEAVE_C4_SOURCE_CONTROL_BINDING_SHA256"),
            )?;
            super::verify_root_identity(&record, path, &held.root)?;
            if record.database != self.database()
                || record.database_oid != self.catalog.installed_source.database_oid
                || record.system_identifier != self.catalog.installed_source.system_identifier
                || hex::encode(held.digest())
                    != self.catalog.installed_source.source_control_binding_sha256
            {
                return Err(BackupError::Invalid("B1 Catalog original control binding"));
            }
            budget
                .lock()
                .map_err(|_| BackupError::Invalid("B1 scan budget"))?
                .charge(held.raw.len() as u64)?;
            Ok((held.root.try_clone()?, held.raw))
        }
        pub(in crate::source) fn isolation(
            &self,
            config: &super::super::SourceBackupConfig,
            require_fresh: bool,
            budget: &Budget,
        ) -> Result<super::super::endpoint::VerifiedSourceIsolation, BackupError> {
            use std::os::unix::fs::MetadataExt;
            if config.backup_id != self.backup_id
                || config.expected_database != self.database()
                || config.expected_compose_project != self.catalog.installed_runtime.compose_project
                || config.isolation_attestation != self.proof_path()
            {
                return Err(BackupError::Invalid(
                    "B1 fixed current source configuration",
                ));
            }
            self.generation_file.recheck(budget)?;
            self.proof.recheck(budget)?;
            self.inspection.recheck(budget)?;
            // Reuse the complete original verifier, including the original
            // require_fresh true/false call and 30-second observed-time gate.
            let isolation = super::super::verify_isolation_attestation_metered(
                config,
                require_fresh,
                Some(budget),
            )?;
            let proof: super::super::IsolationAttestation =
                serde_json::from_slice(&isolation.proof_bytes)?;
            let lock = isolation.lock.metadata()?;
            if isolation.proof_bytes != self.proof.raw
                || isolation.inspection_bytes != self.inspection.raw
                || (lock.dev(), lock.ino()) != (self.generation.lock_dev, self.generation.lock_ino)
                || proof.observed_unix_ms != self.generation.observed_unix_ms
            {
                return Err(BackupError::Invalid(
                    "B1 original generation proof or lock association",
                ));
            }
            Ok(isolation)
        }
        pub(in crate::source) fn require_claim(
            &self,
            claim: super::B1ClaimRefs<'_>,
        ) -> Result<(), BackupError> {
            let super::B1ClaimRefs {
                project,
                container,
                image,
                image_digest,
                daemon,
                network,
                mounts,
                native,
            } = claim;
            let r = &self.catalog.installed_runtime;
            if project != r.compose_project
                || container != r.pg_container_id
                || image != r.admitted_pg_image_id
                || image_digest != r.pg_image_digest
                || daemon != r.daemon_id
                || network != r.internal_network_id
                || mounts != r.capture_mount_recipe_sha256
                || mounts != self.catalog.capture_recipe_sha256
                || native.pid_namespace != r.pg_pid_namespace
                || native.mount_namespace != r.pg_mount_namespace
                || native.postmaster_start_ticks != r.postmaster_start_ticks
            {
                return Err(BackupError::Invalid(
                    "B1 original source claim and Catalog runtime",
                ));
            }
            Ok(())
        }
        pub(in crate::source) fn recheck(
            &self,
            peer: &Arc<B1InstalledSourceProcess>,
            data: &B1SourceIsolationInputData,
            budget: &Budget,
        ) -> Result<(), BackupError> {
            peer.revalidate()
                .map_err(|_| BackupError::Invalid("B1 current source process"))?;
            if data.job_id() != self.job_id
                || data.backup_id() != self.backup_id
                || data.phase() != self.phase
                || data.catalog_sha256() != self.catalog_file.digest()
                || data.generation_binding_sha256() != self.generation_file.digest()
            {
                return Err(BackupError::Invalid(
                    "B1 original input association changed",
                ));
            }
            for file in [
                &self.catalog_file,
                &self.generation_file,
                &self.capture_start,
                &self.proof,
                &self.inspection,
            ] {
                file.recheck(budget)?;
            }
            self.recheck_lock(budget)?;
            peer.revalidate()
                .map_err(|_| BackupError::Invalid("B1 source after current inputs"))?;
            Ok(())
        }
        fn recheck_lock(&self, budget: &Budget) -> Result<(), BackupError> {
            use std::os::fd::AsRawFd;
            budget
                .lock()
                .map_err(|_| BackupError::Invalid("B1 scan budget"))?
                .entries(4)?;
            let named = self
                .generation_file
                .root
                .open_file(&format!("isolation-{}.lock", self.backup_id))?;
            if Stamp::read(&self.generation_lock.metadata()?) != self.generation_lock_stamp
                || Stamp::read(&named.metadata()?) != self.generation_lock_stamp
            {
                return Err(BackupError::Invalid("B1 original generation lock replaced"));
            }
            if unsafe {
                libc::flock(
                    self.generation_lock.as_raw_fd(),
                    libc::LOCK_EX | libc::LOCK_NB,
                )
            } == 0
            {
                unsafe { libc::flock(self.generation_lock.as_raw_fd(), libc::LOCK_UN) };
                return Err(BackupError::Invalid(
                    "B1 original generation driver lock lost",
                ));
            }
            if std::io::Error::last_os_error().raw_os_error() != Some(libc::EWOULDBLOCK) {
                return Err(BackupError::Invalid(
                    "B1 original generation driver lock observation",
                ));
            }
            Ok(())
        }
        /// Retain the original Install issuance after its actual admin close.
        /// This is a private live owner, never reconstructed from copied DATA.
        pub(in crate::source) fn recheck_retained(
            &self,
            peer: &Arc<B1InstalledSourceProcess>,
            budget: &Budget,
        ) -> Result<(), BackupError> {
            peer.revalidate()
                .map_err(|_| BackupError::Invalid("B1 retained original process"))?;
            for file in [
                &self.catalog_file,
                &self.generation_file,
                &self.capture_start,
                &self.proof,
                &self.inspection,
            ] {
                file.recheck(budget)?;
            }
            self.recheck_lock(budget)?;
            peer.revalidate()
                .map_err(|_| BackupError::Invalid("B1 retained original process after inputs"))?;
            Ok(())
        }
        pub(in crate::source) fn require_enrollment(
            &self,
            lease: &RegistryLease,
            enrolled: &EnrolledSource,
        ) -> Result<(), BackupError> {
            let s = &self.catalog.installed_source;
            let g = &enrolled.group;
            lease.recheck()?;
            if !matches!(lease.groups.get(&g.group_id), Some(crate::registry::Group::Source(actual)) if actual == g)
            {
                return Err(BackupError::Invalid(
                    "B1 current original registry source group",
                ));
            }
            {
                let mut scan = lease
                    .budget
                    .lock()
                    .map_err(|_| BackupError::Invalid("B1 scan budget"))?;
                scan.entries(7)?;
                scan.charge((g.group_id.len() + 12 + 4 * (36 + 12) + 3 * 64) as u64)?;
            }
            let authority_raw = &lease
                .metadata
                .get("authority.json")
                .ok_or(BackupError::Invalid("B1 current registry authority bytes"))?
                .0;
            let group_raw = &lease
                .metadata
                .get(&format!("groups/{}.json", g.group_id))
                .ok_or(BackupError::Invalid(
                    "B1 current registry source group bytes",
                ))?
                .0;
            // These exact published canonical lengths come from the current
            // typed lease. Pay clone/serialize/hash/byte-comparison again;
            // earlier read_snapshot charges do not pay this new work.
            let work = authority_raw
                .len()
                .checked_add(group_raw.len())
                .and_then(|n| n.checked_mul(4))
                .ok_or(BackupError::Overflow)?;
            lease
                .budget
                .lock()
                .map_err(|_| BackupError::Invalid("B1 scan budget"))?
                .charge(work as u64)?;
            let authority_canonical = crate::registry::canonical(&lease.authority)?;
            let group_canonical = crate::registry::canonical(g)?;
            if &authority_canonical != authority_raw || &group_canonical != group_raw {
                return Err(BackupError::Invalid(
                    "B1 current canonical registry record bytes",
                ));
            }
            if s.deployment_id != lease.authority.deployment_id
                || s.deployment_id != g.deployment_id
                || s.source_group_id != g.group_id
                || s.database != g.database
                || s.database_oid != u64::from(g.database_oid)
                || s.system_identifier != g.system_identifier
                || s.application_commit != g.application_commit
                || s.application_build_sha256 != g.application_build_sha256
                || s.source_control_binding_sha256 != g.source_binding_sha256
                || s.registry_authority_sha256 != crate::digest(&authority_canonical)
                || s.source_group_record_sha256 != crate::digest(&group_canonical)
                || option_env!("KNOWWEAVE_C4_B1_SOURCE_AUTH_KEY_SHA256")
                    != Some(s.source_auth_public_key_sha256.as_str())
            {
                return Err(BackupError::Invalid(
                    "B1 Catalog original source enrollment",
                ));
            }
            lease.recheck()?;
            let labels = enrolled
                .roots
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>();
            if labels != ["assets", "control", "local_pins", "staging"] {
                return Err(BackupError::Invalid("B1 current four source root records"));
            }
            // Root ADOPTION e35e8476: exact current four-label typed map, full
            // RootRecord values, with no wrapper or alternate hash domain.
            // Pay the bounded clone/serialize/hash work before allocation.
            let mut exact_bytes = 2usize; // braces of the canonical four-label map
            for (index, (label, record)) in enrolled.roots.iter().enumerate() {
                if lease.roots.get(&record.enrollment_id) != Some(record) {
                    return Err(BackupError::Invalid("B1 current source root record"));
                }
                let raw = &lease
                    .metadata
                    .get(&format!("roots/{}.json", record.enrollment_id))
                    .ok_or(BackupError::Invalid("B1 current source root record bytes"))?
                    .0;
                exact_bytes = exact_bytes
                    .checked_add(label.len() + 3 + usize::from(index != 0))
                    .and_then(|n| n.checked_add(raw.len()))
                    .ok_or(BackupError::Overflow)?;
            }
            let work = exact_bytes.checked_mul(4).ok_or(BackupError::Overflow)?;
            lease
                .budget
                .lock()
                .map_err(|_| BackupError::Invalid("B1 scan budget"))?
                .charge(work as u64)?;
            let raw = crate::registry::canonical(&enrolled.roots)?;
            if raw.len() != exact_bytes
                || crate::digest(&raw) != self.catalog.installed_runtime.source_root_records_sha256
            {
                return Err(BackupError::Invalid(
                    "B1 current source root-record aggregate",
                ));
            }
            lease.recheck()?;
            Ok(())
        }
    }

    /// Read-only views of the original Publication phase. These handles are
    /// not an isolation capability, Capture cut, SourceLocalPin, or S factory.
    /// Only the genuine Book passes its own non-Clone opaque captured object.
    pub(crate) struct B1PublicationInputs {
        catalog_file: HeldRecord,
        catalog: Catalog,
        source_start: HeldRecord,
        capture_start: HeldRecord,
        generation_file: HeldRecord,
        captured_file: HeldRecord,
        source_started_ns: u64,
        captured: PublicationCapturedData,
        control: Mutex<Option<HeldRecord>>,
    }

    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PublicationCapturedData {
        schema_version: u64,
        capability: String,
        job_id: Uuid,
        backup_id: Uuid,
        capture_start_sha256: String,
        capture_generation_sha256: String,
        source_group_id: Uuid,
        source_registry_generation_sha256: String,
        manifest_sha256: String,
        captured_roster_sha256: String,
        snapshot_effects_roster_sha256: String,
    }

    impl B1PublicationInputs {
        pub(crate) fn read_original(
            peer: &B1InstalledSourceProcess,
            data: &learning_db::snapshot_broker::B1CapturedInputData,
            budget: &Budget,
        ) -> Result<Self, BackupError> {
            peer.revalidate().map_err(|_| publication_refused())?;
            if option_env!("KNOWWEAVE_C4_B1_SOURCE_PROCESS_ROLE") != Some("publication-source")
                || !crate::registry::v4(&data.job_id().to_string())
                || !crate::registry::v4(&data.backup_id().to_string())
            {
                return Err(publication_refused());
            }
            publication_charge(budget, 4096, 8)?;
            let job_root = PathBuf::from(format!("{OBSERVATION_ROOT}/jobs/{}", data.job_id()));
            let backup_root = job_root.join(format!("backups/{}", data.backup_id()));
            let catalog_file = HeldRecord::open(
                Path::new("/var/lib/knowweave-c4/b1"),
                "source-peer-execution.json".into(),
                16384,
                budget,
            )?;
            prepay_canonical_data(&catalog_file.raw, budget)?;
            let catalog: Catalog = crate::registry::canonical_record(&catalog_file.raw, 16384)?;
            let source_start =
                HeldRecord::open(&job_root, "source-start.json".into(), 4096, budget)?;
            let capture_start =
                HeldRecord::open(&backup_root, "capture-start.json".into(), 16384, budget)?;
            let generation_file = HeldRecord::open(
                &backup_root.join("capture"),
                "generation-binding.json".into(),
                4096,
                budget,
            )?;
            let captured_file =
                HeldRecord::open(&backup_root, "captured-data.json".into(), 4096, budget)?;
            for record in [
                &source_start,
                &capture_start,
                &generation_file,
                &captured_file,
            ] {
                prepay_canonical_data(&record.raw, budget)?;
            }
            // The genuine peer revalidates the complete Root schemas. These
            // canonical projections add joins, never replace that validator.
            let source: serde_json::Value =
                crate::registry::canonical_record(&source_start.raw, 4096)?;
            let capture: serde_json::Value =
                crate::registry::canonical_record(&capture_start.raw, 16384)?;
            let generation: Generation =
                crate::registry::canonical_record(&generation_file.raw, 4096)?;
            let captured: PublicationCapturedData =
                crate::registry::canonical_record(&captured_file.raw, 4096)?;
            let source_started_ns = source["started_monotonic_ns"]
                .as_u64()
                .filter(|n| *n != 0)
                .ok_or_else(publication_refused)?;
            if catalog.schema_version != 1
                || catalog.capability != "b1_source_peer_execution_v1"
                || catalog.profile != "task9-b1-limited-v1"
                || !crate::registry::v4(&catalog.launch_id)
                || !valid_c4_database(&catalog.installed_source.database)
                || source["job_id"] != data.job_id().to_string()
                || source["launch_id"] != catalog.launch_id
                || source["catalog_sha256"] != hex::encode(catalog_file.digest())
                || capture["job_id"] != data.job_id().to_string()
                || capture["backup_id"] != data.backup_id().to_string()
                || capture["source_start_sha256"] != hex::encode(source_start.digest())
                || capture["catalog_sha256"] != hex::encode(catalog_file.digest())
                || capture["mount_recipe_sha256"] != catalog.capture_recipe_sha256
                || capture["mount_recipe_sha256"]
                    != catalog.installed_runtime.capture_mount_recipe_sha256
                || capture["capture_exec_recipe_sha256"]
                    != hash_canonical_value(&catalog.capture_exec_recipe, budget)?
                || generation.schema_version != 1
                || generation.capability != "b1_immutable_isolation_generation_v1"
                || generation.job_id != data.job_id().to_string()
                || generation.backup_id != data.backup_id().to_string()
                || generation.phase != "Capture"
                || !crate::registry::v4(&generation.request_id)
                || generation.capture_start_sha256 != hex::encode(capture_start.digest())
                || captured.source_group_id.to_string() != catalog.installed_source.source_group_id
            {
                return Err(publication_refused());
            }
            let held = Self {
                catalog_file,
                catalog,
                source_start,
                capture_start,
                generation_file,
                captured_file,
                source_started_ns,
                captured,
                control: Mutex::new(None),
            };
            held.recheck_original(peer, data, budget)?;
            Ok(held)
        }

        pub(crate) fn recheck_original(
            &self,
            peer: &B1InstalledSourceProcess,
            data: &learning_db::snapshot_broker::B1CapturedInputData,
            budget: &Budget,
        ) -> Result<(), BackupError> {
            peer.revalidate().map_err(|_| publication_refused())?;
            for held in [
                &self.catalog_file,
                &self.source_start,
                &self.capture_start,
                &self.generation_file,
                &self.captured_file,
            ] {
                held.recheck(budget)?;
            }
            let d = &self.captured;
            if d.schema_version != 1
                || d.capability != "b1_captured_input_data_only_v1"
                || d.job_id != data.job_id()
                || d.backup_id != data.backup_id()
                || self.captured_file.digest() != data.record_sha256()
                || self.capture_start.digest() != data.capture_start_sha256()
                || self.generation_file.digest() != data.capture_generation_sha256()
                || d.capture_start_sha256 != hex::encode(data.capture_start_sha256())
                || d.capture_generation_sha256 != hex::encode(data.capture_generation_sha256())
                || d.source_group_id != data.source_group_id()
                || d.source_registry_generation_sha256
                    != hex::encode(data.source_registry_generation_sha256())
                || d.manifest_sha256 != hex::encode(data.manifest_sha256())
                || d.captured_roster_sha256 != hex::encode(data.captured_roster_sha256())
                || d.snapshot_effects_roster_sha256
                    != hex::encode(data.snapshot_effects_roster_sha256())
            {
                return Err(publication_refused());
            }
            peer.revalidate().map_err(|_| publication_refused())
        }

        pub(crate) fn database(&self) -> &str {
            &self.catalog.installed_source.database
        }
        pub(crate) fn group_id(&self) -> &str {
            &self.catalog.installed_source.source_group_id
        }
        pub(crate) fn backup_id(&self) -> Uuid {
            self.captured.backup_id
        }
        pub(crate) fn require_database(
            &self,
            database: &str,
            oid: i64,
            system: &str,
        ) -> Result<(), BackupError> {
            if database != self.database()
                || u64::try_from(oid).ok() != Some(self.catalog.installed_source.database_oid)
                || system != self.catalog.installed_source.system_identifier
            {
                return Err(publication_refused());
            }
            Ok(())
        }
        pub(crate) fn require_policy_group(
            &self,
            policy_group: &str,
            database: &str,
            binding: &str,
            auth: &str,
            launch: &str,
        ) -> Result<(), BackupError> {
            let source = &self.catalog.installed_source;
            if policy_group != source.source_group_id
                || database != source.database
                || binding != source.source_control_binding_sha256
                || auth != source.source_auth_public_key_sha256
                || launch != self.catalog.launch_id
            {
                return Err(publication_refused());
            }
            Ok(())
        }
        pub(crate) fn require_enrollment(
            &self,
            lease: &RegistryLease,
            enrolled: &EnrolledSource,
        ) -> Result<(), BackupError> {
            require_catalog_enrollment(&self.catalog, lease, enrolled)
        }
        pub(crate) fn require_control(
            &self,
            lease: &RegistryLease,
            source: &EnrolledSource,
        ) -> Result<(), BackupError> {
            let record = &source.roots["control"];
            let mut control = self.control.lock().map_err(|_| publication_refused())?;
            if control.is_none() {
                *control = Some(HeldRecord::open(
                    Path::new(&record.path),
                    "source-binding.json".into(),
                    MAX_BINDING_BYTES as u64,
                    &lease.budget,
                )?);
            }
            let held = control.as_ref().ok_or_else(publication_refused)?;
            if held.parent != Path::new(&record.path) {
                return Err(publication_refused());
            }
            let binding = parse_pinned(
                &held.raw,
                option_env!("KNOWWEAVE_C4_SOURCE_CONTROL_BINDING_SHA256"),
            )?;
            super::verify_root_identity(
                &binding,
                Path::new(&record.path),
                source.handle("control"),
            )?;
            self.require_database(
                &binding.database,
                i64::try_from(binding.database_oid).map_err(|_| publication_refused())?,
                &binding.system_identifier,
            )?;
            if hex::encode(held.digest())
                != self.catalog.installed_source.source_control_binding_sha256
                || held.root.identity()? != source.handle("control").identity()?
            {
                return Err(publication_refused());
            }
            held.recheck(&lease.budget)
        }
        pub(crate) fn conservative_root_deadline(
            &self,
            budget: &Budget,
        ) -> Result<std::time::Instant, BackupError> {
            // Same boot and Root SourceStart were checked in the original ctx.
            // Anchor before native clock IO, never after, so scheduling cannot
            // extend the actual Root 1200-second bound.
            let anchor = std::time::Instant::now();
            publication_charge(budget, 32, 1)?;
            let mut time: libc::timespec = unsafe { std::mem::zeroed() };
            if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let now = u64::try_from(time.tv_sec)
                .ok()
                .and_then(|n| n.checked_mul(1_000_000_000))
                .and_then(|n| {
                    u64::try_from(time.tv_nsec)
                        .ok()
                        .and_then(|v| n.checked_add(v))
                })
                .ok_or_else(publication_refused)?;
            let remaining = publication_root_remaining(self.source_started_ns, now)?;
            anchor
                .checked_add(std::time::Duration::from_nanos(remaining))
                .ok_or(BackupError::Overflow)
        }
    }

    fn publication_refused() -> BackupError {
        BackupError::Invalid("B1 original Publication source view")
    }
    fn publication_charge(budget: &Budget, bytes: u64, entries: usize) -> Result<(), BackupError> {
        let mut scan = budget.lock().map_err(|_| publication_refused())?;
        scan.charge(bytes)?;
        scan.entries(entries)
    }
    fn publication_root_remaining(start: u64, now: u64) -> Result<u64, BackupError> {
        let end = start
            .checked_add(1_200_000_000_000)
            .ok_or(BackupError::Overflow)?;
        if start == 0 || now < start || now >= end {
            return Err(publication_refused());
        }
        Ok(end - now)
    }

    // Shared fixed Catalog/current-lease join. Kept inside this existing child
    // module so neither the old Capture fields nor their parser are exposed.
    fn require_catalog_enrollment(
        catalog: &Catalog,
        lease: &RegistryLease,
        enrolled: &EnrolledSource,
    ) -> Result<(), BackupError> {
        let s = &catalog.installed_source;
        let g = &enrolled.group;
        lease.recheck()?;
        if !matches!(lease.groups.get(&g.group_id), Some(crate::registry::Group::Source(actual)) if actual == g)
        {
            return Err(publication_refused());
        }
        publication_charge(&lease.budget, 4096, 8)?;
        let authority_raw = &lease
            .metadata
            .get("authority.json")
            .ok_or_else(publication_refused)?
            .0;
        let group_raw = &lease
            .metadata
            .get(&format!("groups/{}.json", g.group_id))
            .ok_or_else(publication_refused)?
            .0;
        publication_charge(
            &lease.budget,
            (authority_raw.len() + group_raw.len()) as u64 * 4,
            0,
        )?;
        let authority = crate::registry::canonical(&lease.authority)?;
        let group = crate::registry::canonical(g)?;
        if &authority != authority_raw
            || &group != group_raw
            || s.deployment_id != lease.authority.deployment_id
            || s.deployment_id != g.deployment_id
            || s.source_group_id != g.group_id
            || s.database != g.database
            || s.database_oid != u64::from(g.database_oid)
            || s.system_identifier != g.system_identifier
            || s.application_commit != g.application_commit
            || s.application_build_sha256 != g.application_build_sha256
            || s.source_control_binding_sha256 != g.source_binding_sha256
            || s.registry_authority_sha256 != crate::digest(&authority)
            || s.source_group_record_sha256 != crate::digest(&group)
            || option_env!("KNOWWEAVE_C4_B1_SOURCE_AUTH_KEY_SHA256")
                != Some(s.source_auth_public_key_sha256.as_str())
            || enrolled.roots.keys().map(String::as_str).ne([
                "assets",
                "control",
                "local_pins",
                "staging",
            ])
        {
            return Err(publication_refused());
        }
        let mut expected = 2usize;
        for (i, (label, record)) in enrolled.roots.iter().enumerate() {
            if lease.roots.get(&record.enrollment_id) != Some(record) {
                return Err(publication_refused());
            }
            let raw = &lease
                .metadata
                .get(&format!("roots/{}.json", record.enrollment_id))
                .ok_or_else(publication_refused)?
                .0;
            expected = expected
                .checked_add(label.len() + 3 + usize::from(i != 0))
                .and_then(|n| n.checked_add(raw.len()))
                .ok_or(BackupError::Overflow)?;
        }
        publication_charge(
            &lease.budget,
            (expected as u64)
                .checked_mul(4)
                .ok_or(BackupError::Overflow)?,
            0,
        )?;
        let roots = crate::registry::canonical(&enrolled.roots)?;
        if roots.len() != expected
            || crate::digest(&roots) != catalog.installed_runtime.source_root_records_sha256
        {
            return Err(publication_refused());
        }
        lease.recheck()
    }

    /// Held read-only package + source mirror + v2 adjunct. It deliberately
    /// cannot be converted into the Capture owner's cut or sealed capability.
    pub(crate) struct B1PublicationPinView {
        pin: BackupDir,
        mirror: BackupDir,
        effects: BackupDir,
        companion: BackupDir,
        manifest_file: File,
        manifest_stamp: Stamp,
        manifest_raw: Vec<u8>,
        index_file: File,
        index_stamp: Stamp,
        index_raw: Vec<u8>,
        manifest: crate::BackupManifestV1,
        source_records: Vec<crate::FileRecord>,
        source_bytes: std::collections::BTreeMap<String, Vec<u8>>,
        current_snapshot_raw: Vec<u8>,
        current_snapshot: crate::destination::SnapshotRecord,
        effects_records: Vec<crate::FileRecord>,
        capture: crate::source::snapshot_effects::wire::Capture,
        descriptor: crate::source::snapshot_effects::wire::Descriptor,
    }
    impl B1PublicationPinView {
        pub(crate) fn open_original(
            input: &B1PublicationInputs,
            lease: &RegistryLease,
            source: &EnrolledSource,
            protection: &crate::ProtectionSnapshot,
        ) -> Result<Self, BackupError> {
            use crate::source::snapshot_effects::{files, wire};
            input.require_enrollment(lease, source)?;
            protection.require_same_operation(lease)?;
            publication_charge(&lease.budget, 16384, 32)?;
            let backup = input.backup_id();
            let pin = source
                .handle("local_pins")
                .open_dir(&format!("{backup}.sealed"))?;
            let mirror = source
                .handle("control")
                .open_dir(&format!("{backup}.b1-captured"))?;
            let companion = BackupDir::open_trusted_private_root(Path::new(
                learning_assets::InstalledSnapshotBinding::COMPANION_PATH,
            ))?;
            let effects = companion
                .open_dir("captures")?
                .open_dir(&backup.to_string())?;
            let mut manifest_file = pin.open_file("manifest.json")?;
            let manifest_meta = manifest_file.metadata()?;
            if !manifest_meta.is_file()
                || manifest_meta.uid() != 0
                || manifest_meta.gid() != 0
                || manifest_meta.mode() & 0o7777 != 0o400
                || manifest_meta.nlink() != 1
                || manifest_meta.len() == 0
                || manifest_meta.len() > 64 * 1024 * 1024
            {
                return Err(publication_refused());
            }
            let manifest_stamp = Stamp::read(&manifest_meta);
            // Prepay this new caller's legacy typed manifest/index validation
            // before its parser/canonical tree can allocate. Streaming member
            // byte verification keeps the existing per-read charges as well.
            publication_charge(
                &lease.budget,
                manifest_meta
                    .len()
                    .checked_mul(64)
                    .ok_or(BackupError::Overflow)?,
                512,
            )?;
            let manifest_raw =
                read_held_bytes(&mut manifest_file, manifest_meta.len(), &lease.budget)?;
            let declared: crate::BackupManifestV1 = serde_json::from_slice(&manifest_raw)?;
            publication_charge(&lease.budget, manifest_raw.len() as u64 * 4, 2)?;
            if declared.canonical_bytes()? != manifest_raw
                || crate::digest(&manifest_raw) != input.captured.manifest_sha256
                || Stamp::read(&manifest_file.metadata()?) != manifest_stamp
            {
                return Err(publication_refused());
            }
            publication_charge(&lease.budget, 4096, 4)?;
            let index_file = pin.open_file("asset-index.json")?;
            let index_meta = index_file.metadata()?;
            let index_stamp = Stamp::read(&index_meta);
            let declared_index = declared
                .files
                .iter()
                .find(|r| r.path == "asset-index.json")
                .ok_or_else(publication_refused)?;
            if !index_meta.is_file()
                || index_meta.uid() != 0
                || index_meta.gid() != 0
                || index_meta.mode() & 0o7777 != 0o400
                || index_meta.nlink() != 1
                || index_meta.len() != declared_index.size
                || index_meta.len() == 0
                || index_meta.len() > crate::MAX_ASSET_INDEX_BYTES as u64
            {
                return Err(publication_refused());
            }
            prepay_publication_package(lease, &declared, &manifest_raw, index_meta.len())?;
            let (manifest, index_raw) =
                crate::protection::verify_registered_package(lease, &pin, backup)?;
            if manifest != declared
                || index_raw != publication_pread(&index_file, index_raw.len(), &lease.budget)?
                || Stamp::read(&index_file.metadata()?) != index_stamp
                || Stamp::read(&pin.open_file("asset-index.json")?.metadata()?) != index_stamp
            {
                return Err(publication_refused());
            }
            let budget = &lease.budget;
            // The source mirror has exactly eight fixed files and the exact
            // current four enrolled RootRecord leaves. List before reading.
            let mut names = Vec::new();
            publication_charge(budget, 2048, 16)?;
            for name in [
                "protection-catalog.json",
                "protection-pending.json",
                "protection-retained.json",
                "protection-snapshot.json",
                "source-authority.json",
                "source-generation.json",
                "source-group.json",
                "source-released.json",
            ] {
                names.push(name.to_owned());
            }
            for root in source.roots.values() {
                names.push(format!("roots/{}.json", root.enrollment_id));
            }
            names.sort_unstable();
            let mut source_records = Vec::new();
            let mut source_bytes = std::collections::BTreeMap::new();
            source_records
                .try_reserve_exact(12)
                .map_err(|_| publication_refused())?;
            let mut captured_snapshot = None;
            for name in &names {
                let (raw, _) = publication_sealed_read(
                    &mirror,
                    name,
                    crate::MAX_ASSET_INDEX_BYTES as u64,
                    budget,
                )?;
                publication_charge(budget, raw.len() as u64 * 3, 1)?;
                if name == "protection-snapshot.json" {
                    prepay_canonical_data(&raw, budget)?;
                    captured_snapshot = Some(crate::registry::canonical_record(
                        &raw,
                        crate::MAX_ASSET_INDEX_BYTES as u64,
                    )?);
                }
                source_records.push(crate::FileRecord {
                    path: name.clone(),
                    size: raw.len() as u64,
                    sha256: crate::digest(&raw),
                });
                source_bytes.insert(name.clone(), raw);
            }
            let journal = original_scan_from_budget(lease)?;
            files::exact_tree(&mirror, &source_records, budget, &journal)?;
            publication_charge(budget, 4096, 4)?;
            let (raw, _) =
                publication_sealed_read(&effects, "capture.json", wire::MAX_FILE, budget)?;
            prepay_canonical_data(&raw, budget)?;
            let capture: wire::Capture = crate::registry::canonical_record(&raw, wire::MAX_FILE)?;
            publication_charge(
                budget,
                (raw.len() as u64) * 4 + (capture.files.len() as u64) * 1024,
                capture.files.len() + 1,
            )?;
            let mut effects_records = capture.files.clone();
            effects_records.push(crate::FileRecord {
                path: "capture.json".into(),
                size: raw.len() as u64,
                sha256: crate::digest(&raw),
            });
            effects_records.sort_by(|a, b| a.path.cmp(&b.path));
            publication_roster(&source_records)?;
            publication_roster(&effects_records)?;
            if source_records.len() != 12
                || effects_records.len() + 12 + manifest.files.len() + 1 > 512
                || capture.backup_id != backup
                || capture.source_group_id.to_string() != source.group.group_id
                || capture.manifest_sha256 != input.captured.manifest_sha256
                || capture.source_registry_generation_sha256
                    != input.captured.source_registry_generation_sha256
                || crate::digest(&crate::registry::canonical(&source_records)?)
                    != input.captured.captured_roster_sha256
                || crate::digest(&crate::registry::canonical(&effects_records)?)
                    != input.captured.snapshot_effects_roster_sha256
            {
                return Err(publication_refused());
            }
            files::exact_tree(&effects, &effects_records, budget, &journal)?;
            let descriptor = publication_adjunct(
                &effects,
                &capture,
                &effects_records,
                input,
                source,
                lease,
                &manifest,
            )?;
            if companion.identity()?
                != (descriptor.companion_root_dev, descriptor.companion_root_ino)
            {
                return Err(publication_refused());
            }
            let captured_snapshot = captured_snapshot.ok_or_else(publication_refused)?;
            protection.prepay_publication_views(lease)?;
            let mut current_files = protection.captured_files(backup)?;
            let current_snapshot_raw = current_files
                .remove("protection-snapshot.json")
                .ok_or_else(publication_refused)?;
            prepay_canonical_data(&current_snapshot_raw, budget)?;
            let current_snapshot: crate::destination::SnapshotRecord =
                crate::registry::canonical_record(
                    &current_snapshot_raw,
                    crate::MAX_ASSET_INDEX_BYTES as u64,
                )?;
            // This one producer join depends on genuine opaque Capture DATA,
            // original exclusive create + nonreplace export and exact12 bytes.
            // It is not the later current-S/fence comparison or an ignore ID API.
            publication_historical_export_join(
                input,
                &captured_snapshot,
                &current_snapshot,
                &source_records,
                budget,
            )?;
            let held = Self {
                pin,
                mirror,
                effects,
                companion,
                manifest_file,
                manifest_stamp,
                manifest_raw,
                index_file,
                index_stamp,
                index_raw,
                manifest,
                source_records,
                source_bytes,
                current_snapshot_raw,
                current_snapshot,
                effects_records,
                capture,
                descriptor,
            };
            held.recheck(input, lease, source, protection)?;
            Ok(held)
        }
        pub(crate) fn recheck(
            &self,
            input: &B1PublicationInputs,
            lease: &RegistryLease,
            source: &EnrolledSource,
            protection: &crate::ProtectionSnapshot,
        ) -> Result<(), BackupError> {
            use crate::source::snapshot_effects::files;
            input.require_enrollment(lease, source)?;
            protection.require_same_operation(lease)?;
            publication_charge(
                &lease.budget,
                self.manifest_raw.len() as u64 * 4 + 16384,
                12,
            )?;
            if source
                .handle("local_pins")
                .open_dir(&format!("{}.sealed", input.backup_id()))?
                .identity()?
                != self.pin.identity()?
                || source
                    .handle("control")
                    .open_dir(&format!("{}.b1-captured", input.backup_id()))?
                    .identity()?
                    != self.mirror.identity()?
                || BackupDir::open_trusted_private_root(Path::new(
                    learning_assets::InstalledSnapshotBinding::COMPANION_PATH,
                ))?
                .identity()?
                    != self.companion.identity()?
                || self
                    .companion
                    .open_dir("captures")?
                    .open_dir(&input.backup_id().to_string())?
                    .identity()?
                    != self.effects.identity()?
                || Stamp::read(&self.manifest_file.metadata()?) != self.manifest_stamp
                || Stamp::read(&self.pin.open_file("manifest.json")?.metadata()?)
                    != self.manifest_stamp
            {
                return Err(publication_refused());
            }
            if Stamp::read(&self.index_file.metadata()?) != self.index_stamp
                || Stamp::read(&self.pin.open_file("asset-index.json")?.metadata()?)
                    != self.index_stamp
            {
                return Err(publication_refused());
            }
            prepay_publication_package(
                lease,
                &self.manifest,
                &self.manifest_raw,
                self.index_stamp.bytes,
            )?;
            let (manifest, index) =
                crate::protection::verify_registered_package(lease, &self.pin, input.backup_id())?;
            if manifest != self.manifest
                || manifest.canonical_bytes()? != self.manifest_raw
                || index != self.index_raw
                || index
                    != publication_pread(&self.index_file, self.index_raw.len(), &lease.budget)?
                || Stamp::read(&self.index_file.metadata()?) != self.index_stamp
            {
                return Err(publication_refused());
            }
            // pread preserves the original shared file description offset;
            // a future manifest stream must not be rewound by revalidation.
            let held_raw =
                publication_pread(&self.manifest_file, self.manifest_raw.len(), &lease.budget)?;
            if held_raw != self.manifest_raw
                || Stamp::read(&self.manifest_file.metadata()?) != self.manifest_stamp
            {
                return Err(publication_refused());
            }
            let journal = original_scan_from_budget(lease)?;
            files::exact_tree(&self.mirror, &self.source_records, &lease.budget, &journal)?;
            files::exact_tree(
                &self.effects,
                &self.effects_records,
                &lease.budget,
                &journal,
            )?;
            protection.prepay_publication_views(lease)?;
            let current = protection.captured_files(input.backup_id())?;
            for record in &self.source_records {
                let raw = current.get(&record.path).ok_or_else(publication_refused)?;
                if record.path == "protection-snapshot.json" {
                    prepay_canonical_data(raw, &lease.budget)?;
                    let now: crate::destination::SnapshotRecord =
                        crate::registry::canonical_record(
                            raw,
                            crate::MAX_ASSET_INDEX_BYTES as u64,
                        )?;
                    // Full fresh-S records include the mirror and must remain
                    // exact throughout S; no historical alias filtering here.
                    publication_snapshot_join(&self.current_snapshot, &now)?;
                } else if record.size != raw.len() as u64 || record.sha256 != crate::digest(raw) {
                    return Err(publication_refused());
                }
            }
            protection.prepay_publication_views(lease)?;
            protection.recheck_held()?;
            Ok(())
        }
        pub(crate) fn require_postmaster(&self, observed: &str) -> Result<(), BackupError> {
            if self.descriptor.source_postmaster_start != observed {
                return Err(publication_refused());
            }
            Ok(())
        }
        pub(crate) fn manifest(&self) -> &crate::BackupManifestV1 {
            &self.manifest
        }
        pub(crate) fn manifest_raw_bytes(&self) -> &[u8] {
            &self.manifest_raw
        }
        pub(crate) fn source_bytes(&self) -> &std::collections::BTreeMap<String, Vec<u8>> {
            &self.source_bytes
        }
        pub(crate) fn current_snapshot_bytes(&self) -> &[u8] {
            &self.current_snapshot_raw
        }
        pub(crate) fn source_records(&self) -> &[crate::FileRecord] {
            &self.source_records
        }
        pub(crate) fn package_roster_sha256(
            &self,
            lease: &RegistryLease,
        ) -> Result<String, BackupError> {
            publication_charge(
                &lease.budget,
                (self.manifest.files.len() as u64)
                    .checked_mul(2048)
                    .and_then(|n| n.checked_add(self.manifest_raw.len() as u64))
                    .ok_or(BackupError::Overflow)?,
                self.manifest.files.len() + 1,
            )?;
            let mut records = self.manifest.files.clone();
            records.push(crate::FileRecord {
                path: "manifest.json".into(),
                size: self.manifest_raw.len() as u64,
                sha256: crate::digest(&self.manifest_raw),
            });
            records.sort_by(|a, b| a.path.cmp(&b.path));
            Ok(crate::digest(&crate::registry::canonical(&records)?))
        }
        pub(crate) fn effects_records(&self) -> &[crate::FileRecord] {
            &self.effects_records
        }
        pub(crate) fn capture_data(&self) -> &crate::source::snapshot_effects::wire::Capture {
            &self.capture
        }
        pub(crate) fn open_package_member_original(
            &self,
            index: usize,
            lease: &RegistryLease,
        ) -> Result<File, BackupError> {
            let record = self
                .manifest
                .files
                .get(index)
                .ok_or_else(publication_refused)?;
            publication_charge(
                &lease.budget,
                record.path.len() as u64,
                record.path.split('/').count() * 2 + 3,
            )?;
            let file = publication_open_package_member(&self.pin, &record.path)?;
            publication_require_sealed(&file.metadata()?, record.size)?;
            Ok(file)
        }
        pub(crate) fn open_effect_member_original(
            &self,
            index: usize,
            lease: &RegistryLease,
        ) -> Result<File, BackupError> {
            let record = self
                .effects_records
                .get(index)
                .ok_or_else(publication_refused)?;
            publication_charge(
                &lease.budget,
                record.path.len() as u64,
                record.path.split('/').count() * 2 + 3,
            )?;
            let file = publication_open_member(&self.effects, &record.path)?;
            publication_require_sealed(&file.metadata()?, record.size)?;
            Ok(file)
        }
    }

    fn publication_open_package_member(root: &BackupDir, path: &str) -> Result<File, BackupError> {
        if !crate::source::snapshot_effects::wire::path(path) {
            return Err(publication_refused());
        }
        let mut parts = path.split('/').peekable();
        let mut parent = root.try_clone()?;
        parent.require_sealed_directory()?;
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                return Ok(parent.open_file(part)?);
            }
            parent = parent.open_dir(part)?;
            parent.require_sealed_directory()?;
        }
        Err(publication_refused())
    }
    fn publication_open_member(root: &BackupDir, path: &str) -> Result<File, BackupError> {
        if !crate::source::snapshot_effects::wire::path(path) {
            return Err(publication_refused());
        }
        let mut parts = path.split('/').peekable();
        let mut parent = root.try_clone()?;
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                return Ok(parent.open_file(part)?);
            }
            parent = parent.open_dir(part)?;
            parent.require_private_directory()?;
        }
        Err(publication_refused())
    }
    fn publication_require_sealed(metadata: &Metadata, bytes: u64) -> Result<(), BackupError> {
        if !metadata.is_file()
            || metadata.uid() != 0
            || metadata.mode() & 0o7777 != 0o400
            || metadata.nlink() != 1
            || metadata.len() != bytes
        {
            return Err(publication_refused());
        }
        Ok(())
    }
    fn prepay_publication_package(
        lease: &RegistryLease,
        manifest: &crate::BackupManifestV1,
        raw: &[u8],
        actual_index_bytes: u64,
    ) -> Result<(), BackupError> {
        if manifest.files.len().checked_add(1).is_none_or(|n| n > 512) {
            return Err(publication_refused());
        }
        let bytes = manifest.files.iter().try_fold(raw.len() as u64, |n, r| {
            n.checked_add(r.size).ok_or(BackupError::Overflow)
        })?;
        if bytes > 268_435_456 {
            return Err(BackupError::Capacity("B1 source package profile"));
        }
        let index = manifest
            .files
            .iter()
            .find(|r| r.path == "asset-index.json")
            .ok_or_else(publication_refused)?;
        if index.size != actual_index_bytes
            || actual_index_bytes > crate::MAX_ASSET_INDEX_BYTES as u64
        {
            return Err(publication_refused());
        }
        let work = bytes
            .checked_mul(2)
            .and_then(|n| n.checked_add((raw.len() as u64).checked_mul(64)?))
            .and_then(|n| n.checked_add(actual_index_bytes.checked_mul(64)?))
            .ok_or(BackupError::Overflow)?;
        // The legacy full package verifier streams some sealed files without
        // a registry meter. Reserve those complete reads/hashes before calling
        // it; limits do not promise every maximum-size package fits the budget.
        publication_charge(
            &lease.budget,
            work,
            manifest
                .files
                .len()
                .checked_mul(8)
                .and_then(|n| n.checked_add(32))
                .ok_or(BackupError::Overflow)?,
        )
    }
    fn publication_pread(
        file: &File,
        bytes: usize,
        budget: &Budget,
    ) -> Result<Vec<u8>, BackupError> {
        use std::os::unix::fs::FileExt;
        publication_charge(
            budget,
            (bytes as u64)
                .checked_mul(2)
                .and_then(|n| n.checked_add(1))
                .ok_or(BackupError::Overflow)?,
            2,
        )?;
        let mut raw = vec![0u8; bytes];
        let mut offset = 0usize;
        while offset < bytes {
            let n = file.read_at(&mut raw[offset..], offset as u64)?;
            if n == 0 {
                return Err(publication_refused());
            }
            offset = offset.checked_add(n).ok_or(BackupError::Overflow)?;
        }
        let mut eof = [0u8];
        if file.read_at(&mut eof, bytes as u64)? != 0 {
            return Err(publication_refused());
        }
        Ok(raw)
    }

    fn publication_sealed_read(
        root: &BackupDir,
        name: &str,
        cap: u64,
        budget: &Budget,
    ) -> Result<(Vec<u8>, Stamp), BackupError> {
        if !crate::source::snapshot_effects::wire::path(name) {
            return Err(publication_refused());
        }
        publication_charge(
            budget,
            (name.len() as u64) * 2 + 4096,
            name.split('/').count() * 4 + 4,
        )?;
        let mut directory = root.try_clone()?;
        let mut parts = name.split('/').peekable();
        let mut leaf = None;
        while let Some(part) = parts.next() {
            if parts.peek().is_some() {
                directory = directory.open_dir(part)?;
            } else {
                leaf = Some(part);
            }
        }
        let leaf = leaf.ok_or_else(publication_refused)?;
        let mut file = directory.open_file(leaf)?;
        let meta = file.metadata()?;
        if !meta.is_file()
            || meta.uid() != 0
            || meta.gid() != 0
            || meta.nlink() != 1
            || meta.mode() & 0o7777 != 0o400
            || meta.len() == 0
            || meta.len() > cap
        {
            return Err(publication_refused());
        }
        let stamp = Stamp::read(&meta);
        let raw = read_held_bytes(&mut file, meta.len(), budget)?;
        let mut extra = [0u8];
        publication_charge(budget, 1, 2)?;
        if file.read(&mut extra)? != 0
            || Stamp::read(&file.metadata()?) != stamp
            || Stamp::read(&directory.open_file(leaf)?.metadata()?) != stamp
        {
            return Err(publication_refused());
        }
        Ok((raw, stamp))
    }

    fn publication_adjunct(
        effects: &BackupDir,
        capture: &crate::source::snapshot_effects::wire::Capture,
        records: &[crate::FileRecord],
        input: &B1PublicationInputs,
        source: &EnrolledSource,
        lease: &RegistryLease,
        manifest: &crate::BackupManifestV1,
    ) -> Result<crate::source::snapshot_effects::wire::Descriptor, BackupError> {
        use crate::source::snapshot_effects::{files, verify, wire};
        let journal = original_scan_from_budget(lease)?;
        let read = |name: &str| -> Result<Vec<u8>, BackupError> {
            let record = records
                .iter()
                .find(|r| r.path == name)
                .ok_or_else(publication_refused)?;
            files::read(effects, record, &lease.budget, &journal)
        };
        let descriptor_raw = read("epoch.json")?;
        let ledger_raw = read("ledger.json")?;
        let cut_raw = read("cut.json")?;
        let namespace_raw = read("namespace.json")?;
        let inventory_raw = read("source-rows.json")?;
        for raw in [
            &descriptor_raw,
            &ledger_raw,
            &cut_raw,
            &namespace_raw,
            &inventory_raw,
        ] {
            prepay_canonical_data(raw, &lease.budget)?;
        }
        let descriptor: wire::Descriptor = publication_decode(&descriptor_raw)?;
        let ledger: wire::Ledger = publication_decode(&ledger_raw)?;
        let cut: wire::Cut = publication_decode(&cut_raw)?;
        let namespace: Vec<wire::NamespaceEntry> = publication_decode(&namespace_raw)?;
        let inventory: crate::full_restore::jobs::JobInventory =
            publication_decode(&inventory_raw)?;
        let g = &source.group;
        // This shared checker also hashes the four fixed compiled writer
        // sources (96,867 bytes in this reviewed baseline) and UUID strings.
        // Pay its complete static input/work before the call, independently
        // of Descriptor JSON/read billing. No replacement meter is created.
        publication_charge(&lease.budget, 1_048_576, 16)?;
        crate::source::snapshot_effects::descriptor_matches(&descriptor, g)?;
        if capture.format_version != 2
            || capture.capability != "snapshot_effect_capture_v2"
            || capture.storage_encoding != wire::ENCODING
            || descriptor.format_version != 2
            || descriptor.capability != "snapshot_effect_epoch_v2"
            || descriptor.storage_encoding != wire::ENCODING
            || descriptor.source_group_id.to_string() != g.group_id
            || descriptor.deployment_id.to_string() != g.deployment_id
            || descriptor.source_database != g.database
            || descriptor.source_database_oid != i64::from(g.database_oid)
            || descriptor.source_system_identifier != g.system_identifier
            || descriptor.source_application_commit != g.application_commit
            || descriptor.source_build_sha256 != g.application_build_sha256
            || manifest.source.application_commit != g.application_commit
            || manifest.source.application_build_sha256 != g.application_build_sha256
            || capture.capture_id != cut.capture_id
            || capture.epoch_sha256 != crate::digest(&descriptor_raw)
            || capture.rows_sha256 != crate::digest(&inventory_raw)
            || capture.namespace_sha256 != crate::digest(&namespace_raw)
            || capture.journal_head_sequence != cut.cut_record.sequence
            || capture.journal_head_sha256 != cut.cut_record.sha256
            || capture.cut_event_sha256 != cut.cut_record.sha256
            || capture.backup_id != input.backup_id()
        {
            return Err(publication_refused());
        }
        let mut journal_files = std::collections::BTreeMap::new();
        for record in records.iter().filter(|r| r.path.starts_with("journal/")) {
            publication_charge(&lease.budget, record.path.len() as u64 + 256, 1)?;
            journal_files.insert(
                record.path.clone(),
                files::read(effects, record, &lease.budget, &journal)?,
            );
        }
        let prefix = wire::Prefix::check(
            &journal_files,
            &descriptor_raw,
            descriptor.epoch_id,
            descriptor.binding_id,
            &cut.cut_record,
        )?;
        verify::cut(
            &descriptor,
            &ledger,
            &cut,
            (
                cut.cut_record.sequence,
                hex::decode(&cut.cut_record.sha256)
                    .ok()
                    .and_then(|v| v.try_into().ok())
                    .ok_or_else(publication_refused)?,
            ),
            &descriptor_raw,
            &ledger_raw,
            input.backup_id(),
        )?;
        verify::prefix_cut(
            &prefix,
            &cut,
            &descriptor_raw,
            &ledger_raw,
            &lease.budget,
            &journal,
        )?;
        verify::inventory(
            &inventory,
            &ledger,
            &prefix,
            &namespace,
            &lease.budget,
            &journal,
        )?;
        Ok(descriptor)
    }
    fn publication_decode<T: serde::de::DeserializeOwned + Serialize>(
        raw: &[u8],
    ) -> Result<T, BackupError> {
        // Existing exports use either the closed typed field order or sorted
        // Value encoding. Roundtripping either permitted encoding rejects
        // duplicate keys at all depths; neither form adds a wire alternative.
        let value: T = serde_json::from_slice(raw)?;
        if serde_json::to_vec(&value)? != raw && crate::registry::canonical(&value)? != raw {
            return Err(publication_refused());
        }
        Ok(value)
    }
    fn publication_roster(records: &[crate::FileRecord]) -> Result<(), BackupError> {
        if records.is_empty()
            || records.len() > 512
            || records.windows(2).any(|w| w[0].path >= w[1].path)
            || records.iter().any(|r| {
                !crate::source::snapshot_effects::wire::path(&r.path)
                    || r.size == 0
                    || !crate::valid_digest(&r.sha256)
            })
        {
            return Err(publication_refused());
        }
        Ok(())
    }
    fn publication_snapshot_shape(
        snapshot: &crate::destination::SnapshotRecord,
    ) -> Result<(), BackupError> {
        if snapshot.format_version != 1
            || snapshot.capability != "protection_snapshot_v1"
            || snapshot.scan_bytes > crate::registry::MAX_SCAN_BYTES
            || snapshot.protected_digest_count > 100000
            || snapshot.records.len() > crate::registry::MAX_ENTRIES
            || snapshot.records.windows(2).any(|w| w[0].path >= w[1].path)
        {
            return Err(publication_refused());
        }
        let mut total = 0u64;
        for record in &snapshot.records {
            if record.path.is_empty()
                || record.path.contains(['\\', '\0', ':'])
                || record
                    .path
                    .split('/')
                    .any(|p| p.is_empty() || p == "." || p == "..")
                || !crate::valid_digest(&record.sha256)
                || record.size > crate::MAX_ASSET_INDEX_BYTES as u64
            {
                return Err(publication_refused());
            }
            total = total
                .checked_add(record.size)
                .ok_or(BackupError::Overflow)?;
        }
        if total > snapshot.scan_bytes {
            return Err(publication_refused());
        }
        Ok(())
    }
    fn publication_snapshot_join(
        before: &crate::destination::SnapshotRecord,
        now: &crate::destination::SnapshotRecord,
    ) -> Result<(), BackupError> {
        publication_snapshot_shape(before)?;
        publication_snapshot_shape(now)?;
        if before.records != now.records
            || before.protected_digest_count != now.protected_digest_count
            || before.source_group_id != now.source_group_id
            || before.registry_generation_sha256 != now.registry_generation_sha256
        {
            return Err(publication_refused());
        }
        Ok(())
    }
    fn publication_historical_export_join(
        input: &B1PublicationInputs,
        before: &crate::destination::SnapshotRecord,
        now: &crate::destination::SnapshotRecord,
        source_records: &[crate::FileRecord],
        budget: &Budget,
    ) -> Result<(), BackupError> {
        publication_snapshot_shape(before)?;
        publication_snapshot_shape(now)?;
        publication_roster(source_records)?;
        publication_charge(
            budget,
            (source_records.len() as u64) * 2048,
            source_records.len() * 2,
        )?;
        if source_records.len() != 12
            || crate::digest(&crate::registry::canonical(&source_records)?)
                != input.captured.captured_roster_sha256
            || before.source_group_id != input.group_id()
            || now.source_group_id != input.group_id()
            || before.registry_generation_sha256 != input.captured.source_registry_generation_sha256
            || now.registry_generation_sha256 != input.captured.source_registry_generation_sha256
            || before.protected_digest_count != now.protected_digest_count
        {
            return Err(publication_refused());
        }
        let prefix = format!("control/{}.b1-captured/", input.backup_id());
        if before.records.iter().any(|r| r.path.starts_with(&prefix)) {
            return Err(publication_refused());
        }
        publication_charge(
            budget,
            (now.records.len() as u64)
                .checked_mul(4096)
                .ok_or(BackupError::Overflow)?,
            now.records.len(),
        )?;
        let expected_aliases = source_records
            .iter()
            .map(|record| crate::FileRecord {
                path: format!("{prefix}{}", record.path),
                size: record.size,
                sha256: record.sha256.clone(),
            })
            .collect::<Vec<_>>();
        publication_exact_alias_delta(&before.records, &now.records, &expected_aliases)
    }
    // Pure order/bytes predicate only. Its caller must already hold genuine
    // Capture provenance and the fixed exact12 set, so this cannot create S.
    fn publication_exact_alias_delta(
        before: &[crate::FileRecord],
        now: &[crate::FileRecord],
        expected_aliases: &[crate::FileRecord],
    ) -> Result<(), BackupError> {
        let mut old = before.iter();
        let mut aliases = 0usize;
        for record in now {
            // Exact full-path lookup; no prefix-based suppression.
            if let Some(expected) = expected_aliases.iter().find(|r| r.path == record.path) {
                if record != expected {
                    return Err(publication_refused());
                }
                aliases = aliases.checked_add(1).ok_or(BackupError::Overflow)?;
            } else if old.next() != Some(record) {
                return Err(publication_refused());
            }
        }
        if aliases != expected_aliases.len() || old.next().is_some() {
            return Err(publication_refused());
        }
        Ok(())
    }
    fn original_scan_from_budget(
        lease: &RegistryLease,
    ) -> Result<learning_assets::root_journal::SharedJournalBudget, BackupError> {
        lease
            .budget
            .lock()
            .map_err(|_| publication_refused())?
            .original_b1_scan()
    }

    #[cfg(test)]
    mod first_s_view_tests {
        use super::*;

        #[test]
        fn root_remaining_uses_original_1200_seconds_and_rejects_clock_reversal() {
            let start = 7;
            assert_eq!(
                publication_root_remaining(start, start).unwrap(),
                1_200_000_000_000
            );
            assert_eq!(
                publication_root_remaining(start, start + 1_200_000_000_000 - 1).unwrap(),
                1
            );
            assert!(publication_root_remaining(start, start - 1).is_err());
            assert!(publication_root_remaining(start, start + 1_200_000_000_000).is_err());
            assert!(publication_root_remaining(0, 1).is_err());
            assert!(publication_root_remaining(u64::MAX, u64::MAX).is_err());
        }

        #[test]
        fn publication_roster_is_closed_sorted_and_duplicate_free_data_only() {
            let one = crate::FileRecord {
                path: "source-authority.json".into(),
                size: 2,
                sha256: "a".repeat(64),
            };
            assert!(publication_roster(std::slice::from_ref(&one)).is_ok());
            assert!(publication_roster(&[one.clone(), one]).is_err());
            assert!(
                publication_roster(&[crate::FileRecord {
                    path: "../authority.json".into(),
                    size: 2,
                    sha256: "a".repeat(64)
                }])
                .is_err()
            );
            assert!(
                publication_roster(&[crate::FileRecord {
                    path: "authority.json".into(),
                    size: 0,
                    sha256: "a".repeat(64)
                }])
                .is_err()
            );
            assert!(publication_roster(&[]).is_err());
        }

        #[test]
        fn exact_alias_data_comparison_rejects_extra_prefix_peers_and_changed_hashes() {
            let record = |path: &str| crate::FileRecord {
                path: path.into(),
                size: 2,
                sha256: "a".repeat(64),
            };
            let old = vec![record("registry/authority.json")];
            let aliases = vec![record("control/id.b1-captured/a.json")];
            let now = vec![aliases[0].clone(), old[0].clone()];
            assert!(publication_exact_alias_delta(&old, &now, &aliases).is_ok());
            let extra = vec![
                aliases[0].clone(),
                record("control/id.b1-captured/extra.json"),
                old[0].clone(),
            ];
            assert!(publication_exact_alias_delta(&old, &extra, &aliases).is_err());
            let mut changed = now.clone();
            changed[0].sha256 = "b".repeat(64);
            assert!(publication_exact_alias_delta(&old, &changed, &aliases).is_err());
            assert!(publication_exact_alias_delta(&old, &old, &aliases).is_err());
        }

        #[test]
        fn fresh_snapshot_comparison_keeps_all_records_while_freezing_old_counter_bytes() {
            let record = crate::FileRecord {
                path: "control/id.b1-captured/a.json".into(),
                size: 2,
                sha256: "a".repeat(64),
            };
            let before = crate::destination::SnapshotRecord {
                format_version: 1,
                capability: "protection_snapshot_v1".into(),
                source_group_id: "group".into(),
                registry_generation_sha256: "b".repeat(64),
                records: vec![record],
                protected_digest_count: 1,
                scan_bytes: 64,
            };
            let frozen = crate::registry::canonical(&before).unwrap();
            let mut now = before.clone();
            now.scan_bytes = 128;
            assert!(publication_snapshot_join(&before, &now).is_ok());
            assert_eq!(crate::registry::canonical(&before).unwrap(), frozen);
            now.records[0].size = 3;
            assert!(publication_snapshot_join(&before, &now).is_err());
        }

        #[test]
        fn publication_typed_or_sorted_roundtrip_never_accepts_duplicate_json() {
            #[derive(Serialize, Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Closed {
                z: u64,
                a: u64,
            }
            assert!(publication_decode::<Closed>(br#"{"z":1,"a":2}"#).is_ok());
            assert!(publication_decode::<Closed>(br#"{"a":2,"z":1}"#).is_ok());
            assert!(publication_decode::<Closed>(br#"{"z":1,"a":2,"z":1}"#).is_err());
            assert!(publication_decode::<Closed>(br#"{"z":1,"a":2,"extra":0}"#).is_err());
            assert!(publication_decode::<Closed>(br#"{ "z":1,"a":2}"#).is_err());
        }
    }

    #[cfg(test)]
    mod data_budget_tests {
        use super::*;
        use learning_assets::root_journal::SharedJournalBudget;

        #[test]
        fn canonical_prepayment_counts_structure_outside_strings_on_original_meter() {
            let backing = SharedJournalBudget::default();
            let budget = Arc::new(Mutex::new(ScanBudget::from_b1_scan(backing.clone())));
            let raw = br#"{"x":["{,:","ok"]}"#;
            prepay_canonical_data(raw, &budget).unwrap();
            let actual = backing.usage().unwrap();
            assert_eq!(actual.charged_bytes, raw.len() as u64 * 65);
            assert_eq!(actual.charged_entries, 4);
            assert_eq!(budget.lock().unwrap().bytes, actual.charged_bytes);
        }

        #[test]
        fn exhausted_original_meter_refuses_new_json_wrapper_without_reset() {
            let backing = SharedJournalBudget::default();
            backing.charge(512 * 1024 * 1024 - 1, 0).unwrap();
            let first = Arc::new(Mutex::new(ScanBudget::from_b1_scan(backing.clone())));
            assert!(prepay_canonical_data(br#"{"x":[]}"#, &first).is_err());
            assert!(backing.usage().unwrap().exhausted);
            let later = Arc::new(Mutex::new(ScanBudget::from_b1_scan(backing.clone())));
            assert!(prepay_canonical_data(b"[]", &later).is_err());
            assert_eq!(later.lock().unwrap().bytes, 0);
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SourceBinding {
    format_version: u32,
    capability: String,
    binding_id: String,
    control_path: String,
    control_dev: u64,
    control_ino: u64,
    database: String,
    database_oid: u64,
    system_identifier: String,
}

fn canonical_path(path: &str) -> bool {
    path.starts_with('/')
        && path != "/"
        && !path.contains('\0')
        && path[1..]
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn private_binding_file(uid: u32, mode: u32, links: u64, length: u64) -> bool {
    uid == 0
        && mode & 0o7777 == 0o600
        && links == 1
        && length > 0
        && length <= MAX_BINDING_BYTES as u64
}

fn parse_pinned(bytes: &[u8], pin: Option<&str>) -> Result<SourceBinding, BackupError> {
    let pin = pin.ok_or(BackupError::Invalid(
        "source binding compile-time pin missing",
    ))?;
    if pin.len() != 64
        || !pin
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || bytes.is_empty()
        || bytes.len() > MAX_BINDING_BYTES
        || format!("{:x}", Sha256::digest(bytes)) != pin
    {
        return Err(BackupError::Invalid("source binding digest or length"));
    }
    let record: SourceBinding = serde_json::from_slice(bytes)?;
    // Value object serialization sorts keys, independently of struct order.
    if serde_json::to_vec(&serde_json::to_value(&record)?)? != bytes {
        return Err(BackupError::Invalid("source binding noncanonical bytes"));
    }
    let id = Uuid::parse_str(&record.binding_id)
        .map_err(|_| BackupError::Invalid("source binding UUID"))?;
    let system_identifier = record
        .system_identifier
        .parse::<u64>()
        .map_err(|_| BackupError::Invalid("source binding system identifier"))?;
    if record.format_version != 1
        || record.capability != "source_control_binding_v1"
        || id.to_string() != record.binding_id
        || id.get_variant() != Variant::RFC4122
        || id.get_version() != Some(Version::Random)
        || !canonical_path(&record.control_path)
        || record.control_dev == 0
        || record.control_ino == 0
        || !valid_c4_database(&record.database)
        || record.database_oid == 0
        || record.database_oid > u64::from(u32::MAX)
        || system_identifier == 0
        || system_identifier.to_string() != record.system_identifier
    {
        return Err(BackupError::Invalid("source binding identity fields"));
    }
    Ok(record)
}

#[cfg(target_os = "linux")]
use super::admission::SourceAdmission;
#[cfg(target_os = "linux")]
use learning_assets::backup_fs::BackupDir;
#[cfg(target_os = "linux")]
use std::{io::Read, os::unix::fs::MetadataExt, path::Path};

#[cfg(target_os = "linux")]
fn read_control_root(path: &Path) -> Result<(BackupDir, Vec<u8>), BackupError> {
    let path_text = path
        .to_str()
        .ok_or(BackupError::Invalid("source binding control path UTF-8"))?;
    if !canonical_path(path_text) {
        return Err(BackupError::Invalid("source binding control path"));
    }
    let root = BackupDir::open_trusted_private_root(path)?;
    let file = root.open_file("source-binding.json")?;
    let metadata = file.metadata()?;
    if !private_binding_file(
        metadata.uid(),
        metadata.mode(),
        metadata.nlink(),
        metadata.len(),
    ) {
        return Err(BackupError::Invalid("source binding private file"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BINDING_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BINDING_BYTES {
        return Err(BackupError::Invalid("source binding file length"));
    }
    Ok((root, bytes))
}

#[cfg(target_os = "linux")]
fn verify_root_identity(
    record: &SourceBinding,
    path: &Path,
    root: &BackupDir,
) -> Result<(), BackupError> {
    if path.to_str() != Some(record.control_path.as_str())
        || root.identity()? != (record.control_dev, record.control_ino)
    {
        return Err(BackupError::Invalid("source binding control root identity"));
    }
    Ok(())
}

/// All SQL identity observations use the session that already owns admission.
/// The caller retains this root for every scan and journal/recovery write.
#[cfg(target_os = "linux")]
pub(crate) async fn admit_control_root(
    admission: &mut SourceAdmission,
    path: &Path,
) -> Result<BackupDir, BackupError> {
    #[cfg(feature = "offhost-b1")]
    let (root, bytes) = match admission.read_b1_control_root(path)? {
        Some(original) => original,
        None => read_control_root(path)?,
    };
    #[cfg(not(feature = "offhost-b1"))]
    let (root, bytes) = read_control_root(path)?;
    let record = parse_pinned(
        &bytes,
        option_env!("KNOWWEAVE_C4_SOURCE_CONTROL_BINDING_SHA256"),
    )?;
    verify_root_identity(&record, path, &root)?;
    admission.revalidate()?;
    let (database, database_oid, system_identifier): (String, i64, String) = sqlx::query_as(
        "SELECT current_database()::text, oid::bigint, (pg_catalog.pg_control_system()).system_identifier::text FROM pg_catalog.pg_database WHERE datname=current_database()",
    )
    .fetch_one(admission.connection())
    .await?;
    admission.revalidate()?;
    if database != record.database
        || database != admission.database()
        || database_oid <= 0
        || database_oid as u64 != record.database_oid
        || system_identifier != record.system_identifier
    {
        return Err(BackupError::Invalid("source binding database identity"));
    }
    Ok(root)
}

/// Future root-driver fixtures must independently issue/pin before compilation;
/// this helper only consumes that authority, never creates or registers it.
#[cfg(all(test, target_os = "linux"))]
pub(super) fn preissued_fixture(root_path: &Path) -> (BackupDir, Vec<u8>) {
    let (root, bytes) =
        read_control_root(root_path).expect("independently preissued binding required");
    let record = parse_pinned(
        &bytes,
        option_env!("KNOWWEAVE_C4_SOURCE_CONTROL_BINDING_SHA256"),
    )
    .expect("fixture requires independent binding pin before compilation");
    verify_root_identity(&record, root_path, &root).expect("preissued fixture control identity");
    let lease = crate::ManagementRegistry::open_installed()
        .expect("independently preissued registry required")
        .try_lock()
        .expect("fixture registry admission");
    let enrolled = lease
        .admit_control_source(&root, &record.database)
        .expect("fixture enrolled original group");
    let proof = BackupDir::open_trusted_private_root(root_path.parent().expect("source root"))
        .expect("source root")
        .open_dir("proofs")
        .expect("fixture receipt root");
    let (receipt, _) =
        crate::registry::read(&proof, "registry-provisioning.json", 16384, &lease.budget)
            .expect("independent registry issuer receipt required");
    let receipt: serde_json::Value = crate::registry::canonical_record(&receipt, 16384)
        .expect("canonical registry issuer receipt");
    assert_eq!(
        receipt["source_group_id"].as_str(),
        Some(enrolled.group_id())
    );
    assert_eq!(
        receipt["source_binding_sha256"].as_str(),
        Some(enrolled.group.source_binding_sha256.as_str())
    );
    assert_eq!(
        receipt["authority_sha256"].as_str(),
        Some(crate::digest(&lease.metadata["authority.json"].0).as_str())
    );
    assert_eq!(
        receipt["application_build_sha256"].as_str(),
        option_env!("KNOWWEAVE_BUILD_ID_SHA256")
    );
    assert_eq!(
        receipt["application_commit"].as_str(),
        option_env!("KNOWWEAVE_SOURCE_COMMIT")
    );
    assert_eq!(receipt["issuer_euid"], serde_json::json!(0));
    assert_eq!(receipt["state"], serde_json::json!("READBACK_DURABLE"));
    assert_eq!(
        root.list().unwrap(),
        vec!["source-binding.json"],
        "fresh control contains only independent binding"
    );
    (root, bytes)
}
#[cfg(all(test, target_os = "linux"))]
pub(super) fn fixture_pending(root: &BackupDir, database: &str, id: Uuid) {
    let lease = crate::ManagementRegistry::open_installed()
        .unwrap()
        .try_lock()
        .unwrap();
    let enrolled = lease.admit_control_source(root, database).unwrap();
    crate::begin_capture_protection(&lease, &enrolled, id).unwrap();
}

#[cfg(all(test, target_os = "linux"))]
#[derive(Debug, PartialEq, Eq)]
pub(super) struct FixtureBindingInventory {
    root: (u32, u32, u64, u64),
    entries: Vec<String>,
    file: (u32, u32, u64, u64, u64),
    bytes: Vec<u8>,
}

#[cfg(all(test, target_os = "linux"))]
pub(super) fn fixture_inventory(path: &Path) -> Result<FixtureBindingInventory, BackupError> {
    let (root, bytes) = read_control_root(path)?;
    let root_meta = std::fs::symlink_metadata(path)?;
    let file_meta = root.open_file("source-binding.json")?.metadata()?;
    Ok(FixtureBindingInventory {
        root: (
            root_meta.uid(),
            root_meta.mode(),
            root_meta.dev(),
            root_meta.ino(),
        ),
        entries: root.list()?,
        file: (
            file_meta.uid(),
            file_meta.mode(),
            file_meta.dev(),
            file_meta.ino(),
            file_meta.nlink(),
        ),
        bytes,
    })
}

#[cfg(test)]
mod parser_tests {
    use super::*;

    // Literal, independently specified canonical v1 record; test pins are pure
    // verifier inputs and do not replace the public entry's compiled authority.
    const CANONICAL: &str = concat!(
        "{\"binding_id\":\"11111111-1111-4111-8111-111111111111\",",
        "\"capability\":\"source_control_binding_v1\",\"control_dev\":1,",
        "\"control_ino\":2,\"control_path\":\"/var/lib/knowweave-source/control\",",
        "\"database\":\"learning_backup_c4_task3_22222222-2222-4222-8222-222222222222\",",
        "\"database_oid\":3,\"format_version\":1,\"system_identifier\":\"4\"}"
    );

    fn verify(bytes: &[u8]) -> Result<SourceBinding, BackupError> {
        let pin = format!("{:x}", Sha256::digest(bytes));
        parse_pinned(bytes, Some(&pin))
    }

    #[test]
    fn accepts_independently_specified_canonical_record() {
        let record = verify(CANONICAL.as_bytes()).unwrap();
        assert_eq!(record.database_oid, 3);
        assert_eq!(record.system_identifier, "4");
    }

    #[test]
    fn refuses_missing_malformed_or_mismatched_pin_and_size() {
        assert!(parse_pinned(CANONICAL.as_bytes(), None).is_err());
        for pin in ["", "a", &"A".repeat(64), &"0".repeat(64)] {
            assert!(parse_pinned(CANONICAL.as_bytes(), Some(pin)).is_err());
        }
        assert!(verify(&[]).is_err());
        assert!(verify(&vec![b' '; MAX_BINDING_BYTES + 1]).is_err());
    }

    #[test]
    fn refuses_non_root_wide_mode_linked_or_unbounded_file_metadata() {
        assert!(private_binding_file(0, 0o100600, 1, 4096));
        for (uid, mode, links, length) in [
            (1000, 0o600, 1, 1),
            (0, 0o640, 1, 1),
            (0, 0o660, 1, 1),
            (0, 0o4600, 1, 1),
            (0, 0o600, 2, 1),
            (0, 0o600, 1, 0),
            (0, 0o600, 1, 4097),
        ] {
            assert!(!private_binding_file(uid, mode, links, length));
        }
    }

    #[test]
    fn refuses_noncanonical_json_and_unknown_or_duplicate_fields() {
        let reordered = CANONICAL.replacen(
            "{\"binding_id\":\"11111111-1111-4111-8111-111111111111\",\"capability\":\"source_control_binding_v1\",",
            "{\"capability\":\"source_control_binding_v1\",\"binding_id\":\"11111111-1111-4111-8111-111111111111\",",
            1,
        );
        for bytes in [
            format!("{CANONICAL}\n"),
            CANONICAL.replacen(':', ": ", 1),
            reordered,
            CANONICAL.replacen('{', "{\"extra\":0,", 1),
            CANONICAL.replacen('{', "{\"format_version\":1,", 1),
            CANONICAL.replace("\"format_version\":1", "\"format_version\":1.0"),
            CANONICAL.replace("\"control_dev\":1,", ""),
        ] {
            assert!(verify(bytes.as_bytes()).is_err());
        }
    }

    #[test]
    fn refuses_unsupported_or_noncanonical_identity_fields() {
        let cases = [
            ("\"format_version\":1", "\"format_version\":2"),
            ("source_control_binding_v1", "source_control_binding_v2"),
            (
                "11111111-1111-4111-8111-111111111111",
                "11111111-1111-1111-8111-111111111111",
            ),
            (
                "11111111-1111-4111-8111-111111111111",
                "11111111-1111-4111-7111-111111111111",
            ),
            ("\"control_dev\":1", "\"control_dev\":0"),
            ("\"control_ino\":2", "\"control_ino\":0"),
            ("\"database_oid\":3", "\"database_oid\":0"),
            ("\"database_oid\":3", "\"database_oid\":4294967296"),
            ("\"control_dev\":1", "\"control_dev\":18446744073709551616"),
            ("\"system_identifier\":\"4\"", "\"system_identifier\":\"0\""),
            (
                "\"system_identifier\":\"4\"",
                "\"system_identifier\":\"04\"",
            ),
            (
                "\"system_identifier\":\"4\"",
                "\"system_identifier\":\"18446744073709551616\"",
            ),
            (
                "learning_backup_c4_task3_22222222-2222-4222-8222-222222222222",
                "learning",
            ),
        ];
        for (from, to) in cases {
            assert!(verify(CANONICAL.replace(from, to).as_bytes()).is_err());
        }
        for path in [
            "relative",
            "/",
            "/var//control",
            "/var/./control",
            "/var/../control",
            "/var/control/",
        ] {
            assert!(
                verify(
                    CANONICAL
                        .replace("/var/lib/knowweave-source/control", path)
                        .as_bytes()
                )
                .is_err()
            );
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod filesystem_tests {
    use super::*;
    use crate::{GatePhase, SourceGateJournal};
    use std::{
        fs::{DirBuilder, OpenOptions, Permissions},
        io::Write,
        os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt, symlink},
        path::PathBuf,
    };

    struct Fixture {
        parent: PathBuf,
        control: PathBuf,
    }

    impl Fixture {
        fn new() -> Option<Self> {
            // The acceptance builder's only writable, trusted test area.
            // Never use the enrolled source-control volume for library tests.
            if unsafe { libc::geteuid() } != 0 || !Path::new("/target").is_dir() {
                return None;
            }
            let parent = Path::new("/target").join(format!("c4-binding-fs-{}", Uuid::new_v4()));
            DirBuilder::new().mode(0o700).create(&parent).unwrap();
            let control = parent.join("control");
            DirBuilder::new().mode(0o700).create(&control).unwrap();
            BackupDir::open_trusted_private_root(&control).unwrap();
            Some(Self { parent, control })
        }

        fn issue_test_record(&self) -> Vec<u8> {
            let root = BackupDir::open_trusted_private_root(&self.control).unwrap();
            let (dev, ino) = root.identity().unwrap();
            let bytes = serde_json::to_vec(&serde_json::json!({
                "format_version":1,"capability":"source_control_binding_v1",
                "binding_id":"11111111-1111-4111-8111-111111111111",
                "control_path":self.control.to_str().unwrap(),"control_dev":dev,"control_ino":ino,
                "database":"learning_backup_c4_task3_22222222-2222-4222-8222-222222222222",
                "database_oid":3,"system_identifier":"4"
            }))
            .unwrap();
            let mut file = root.create_file("source-binding.json").unwrap();
            file.write_all(&bytes).unwrap();
            file.sync_all().unwrap();
            bytes
        }

        fn test_record(bytes: &[u8]) -> SourceBinding {
            // Pure verifier injection for filesystem unit tests only. The
            // four live public gates use exclusively the independent build pin.
            let pin = format!("{:x}", Sha256::digest(bytes));
            parse_pinned(bytes, Some(&pin)).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            assert_eq!(self.parent.parent(), Some(Path::new("/target")));
            assert!(
                self.parent
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with("c4-binding-fs-")
            );
            std::fs::remove_dir_all(&self.parent).unwrap();
        }
    }

    #[test]
    fn refuses_missing_linked_wide_oversized_or_non_file_binding() {
        let Some(fixture) = Fixture::new() else {
            return;
        };
        assert!(read_control_root(&fixture.control).is_err());
        let bytes = fixture.issue_test_record();
        let binding_path = fixture.control.join("source-binding.json");
        let (root, actual) = read_control_root(&fixture.control).unwrap();
        assert_eq!(actual, bytes);
        verify_root_identity(&Fixture::test_record(&bytes), &fixture.control, &root).unwrap();
        std::fs::set_permissions(&binding_path, Permissions::from_mode(0o640)).unwrap();
        assert!(read_control_root(&fixture.control).is_err());
        std::fs::set_permissions(&binding_path, Permissions::from_mode(0o600)).unwrap();
        let linked = fixture.parent.join("hard-link");
        std::fs::hard_link(&binding_path, &linked).unwrap();
        assert!(read_control_root(&fixture.control).is_err());
        std::fs::remove_file(&linked).unwrap();
        std::fs::rename(&binding_path, &linked).unwrap();
        symlink(&linked, &binding_path).unwrap();
        assert!(read_control_root(&fixture.control).is_err());
        std::fs::remove_file(&binding_path).unwrap();
        DirBuilder::new().mode(0o700).create(&binding_path).unwrap();
        assert!(read_control_root(&fixture.control).is_err());
        std::fs::remove_dir(&binding_path).unwrap();
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&binding_path)
            .unwrap();
        file.write_all(&vec![b'x'; MAX_BINDING_BYTES + 1]).unwrap();
        file.sync_all().unwrap();
        assert!(read_control_root(&fixture.control).is_err());
        assert_eq!(
            BackupDir::open_trusted_private_root(&fixture.control)
                .unwrap()
                .list()
                .unwrap(),
            vec!["source-binding.json"]
        );
    }

    #[test]
    fn refuses_untrusted_ancestors_symlinks_and_wide_leaf() {
        let Some(fixture) = Fixture::new() else {
            return;
        };
        fixture.issue_test_record();
        std::fs::set_permissions(&fixture.parent, Permissions::from_mode(0o770)).unwrap();
        assert!(read_control_root(&fixture.control).is_err());
        std::fs::set_permissions(&fixture.parent, Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(&fixture.control, Permissions::from_mode(0o750)).unwrap();
        assert!(read_control_root(&fixture.control).is_err());
        std::fs::set_permissions(&fixture.control, Permissions::from_mode(0o700)).unwrap();
        let alias = fixture.parent.join("alias");
        symlink(&fixture.control, &alias).unwrap();
        assert!(read_control_root(&alias).is_err());
        assert!(read_control_root(&fixture.parent.join("control/.")).is_err());
        assert!(read_control_root(&fixture.parent.join("control/../control")).is_err());
        assert_eq!(
            BackupDir::open_trusted_private_root(&fixture.control)
                .unwrap()
                .list()
                .unwrap(),
            vec!["source-binding.json"]
        );
    }

    #[test]
    fn copied_record_and_original_path_with_replaced_inode_are_refused() {
        let Some(fixture) = Fixture::new() else {
            return;
        };
        let bytes = fixture.issue_test_record();
        let record = Fixture::test_record(&bytes);
        let alternate = fixture.parent.join("alternate");
        DirBuilder::new().mode(0o700).create(&alternate).unwrap();
        let root = BackupDir::open_trusted_private_root(&alternate).unwrap();
        let mut file = root.create_file("source-binding.json").unwrap();
        file.write_all(&bytes).unwrap();
        assert!(verify_root_identity(&record, &alternate, &root).is_err());
        let moved = fixture.parent.join("old-control");
        std::fs::rename(&fixture.control, &moved).unwrap();
        DirBuilder::new()
            .mode(0o700)
            .create(&fixture.control)
            .unwrap();
        let replacement = BackupDir::open_trusted_private_root(&fixture.control).unwrap();
        let mut file = replacement.create_file("source-binding.json").unwrap();
        file.write_all(&bytes).unwrap();
        assert!(verify_root_identity(&record, &fixture.control, &replacement).is_err());
        assert_eq!(replacement.list().unwrap(), vec!["source-binding.json"]);
    }

    #[test]
    fn renamed_bound_root_retains_scan_start_and_recovery_authority() {
        let Some(fixture) = Fixture::new() else {
            return;
        };
        let bytes = fixture.issue_test_record();
        let (root, _) = read_control_root(&fixture.control).unwrap();
        verify_root_identity(&Fixture::test_record(&bytes), &fixture.control, &root).unwrap();
        let old_id = Uuid::new_v4();
        SourceGateJournal::start_in(&root, old_id).unwrap();
        let moved = fixture.parent.join("retained-control");
        std::fs::rename(&fixture.control, &moved).unwrap();
        DirBuilder::new()
            .mode(0o700)
            .create(&fixture.control)
            .unwrap();
        let replacement = BackupDir::open_trusted_private_root(&fixture.control).unwrap();
        // Mutation caught: scan or recover follows the now-replaced pathname.
        assert!(super::super::ensure_no_unfinished_journal(&root, &fixture.control).is_err());
        assert_eq!(
            SourceGateJournal::recover_in(&root, old_id)
                .unwrap()
                .record()
                .phase(),
            GatePhase::Intent
        );
        let new_id = Uuid::new_v4();
        SourceGateJournal::start_in(&root, new_id).unwrap();
        let recovery = root
            .create_dir(&format!("{old_id}.release-recovery"))
            .unwrap();
        let mut file = recovery.create_file("closed.json").unwrap();
        file.write_all(b"synthetic-handle-relative-evidence")
            .unwrap();
        file.sync_all().unwrap();
        root.sync().unwrap();
        assert!(replacement.list().unwrap().is_empty());
        assert!(
            moved
                .join(format!("{new_id}.control/intent.json"))
                .is_file()
        );
        assert_eq!(
            std::fs::read(moved.join(format!("{old_id}.release-recovery/closed.json"))).unwrap(),
            b"synthetic-handle-relative-evidence"
        );
    }
}

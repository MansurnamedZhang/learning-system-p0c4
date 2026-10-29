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
pub(super) use b1::{B1InstallCutoverObservation, B1SourceInputs, prepay_canonical_data};

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
    pub(in crate::source) fn prepay_canonical_data(
        raw: &[u8],
        budget: &Budget,
    ) -> Result<(), BackupError> {
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

//! A bounded, read-only management observation. The Docker transport never
//! enters RootSnapshotBroker or a work client. This is the same-Root controller
//! profile, not a claim that this management process is a sandboxed broker.
use super::{
    files::{Budget, charge},
    wire::{Descriptor, hash, invalid},
};
use crate::{BackupError, SourceBackupConfig, registry::RootRecord};
use learning_assets::root_journal::SharedJournalBudget;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Read,
    os::unix::fs::MetadataExt,
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::io::AsyncReadExt;

const DOCKER: &str = "/usr/bin/docker";
const MAX_CONTAINERS: usize = 1024;
// No Config.Env, command, labels outside this allowlist, or secret contents.
const INSPECT: &str = r#"{"Id":{{json .Id}},"Image":{{json .Image}},"State":{"Running":{{json .State.Running}},"Pid":{{json .State.Pid}},"StartedAt":{{json .State.StartedAt}}},"RestartCount":{{json .RestartCount}},"Config":{"Labels":{"com.docker.compose.project":{{json (index .Config.Labels "com.docker.compose.project")}},"com.docker.compose.service":{{json (index .Config.Labels "com.docker.compose.service")}}}},"Mounts":{{json .Mounts}}}"#;

/// Actual capture-namespace observations only. The original Root issuer owns
/// the host all-container before/after object and C11's authenticated record.
/// No Docker invocation, caller CID, host process path or hash-as-observation
/// is used by this independent local half.
#[cfg(feature = "offhost-b1")]
pub(super) struct B1LocalMounts {
    mountinfo: Vec<u8>,
    roots: Vec<(
        std::path::PathBuf,
        learning_assets::backup_fs::BackupDir,
        (u64, u64),
    )>,
    companion: std::fs::File,
    export: std::fs::File,
    companion_identity: (u64, u64),
    export_identity: (u64, u64),
    deadline: Option<Instant>,
}

#[cfg(feature = "offhost-b1")]
impl B1LocalMounts {
    fn observe_mountinfo(budget: &Budget) -> Result<Vec<u8>, BackupError> {
        const CAP: usize = 1024 * 1024;
        let mut scan = budget.lock().map_err(|_| invalid())?;
        scan.entries(8)?;
        // The fixed buffer allocation and the complete bounded read allowance
        // are separate work; reserve both before opening the actual proc file.
        scan.charge(2 * (CAP as u64 + 1))?;
        drop(scan);
        let mut raw = vec![0u8; CAP + 1];
        let mut input = std::fs::File::open("/proc/self/mountinfo")?;
        let mut used = 0;
        loop {
            if used == raw.len() {
                return Err(invalid());
            }
            let count = input.read(&mut raw[used..])?;
            if count == 0 {
                break;
            }
            used += count;
        }
        if used > CAP {
            return Err(invalid());
        }
        raw.truncate(used);
        Ok(raw)
    }
    fn require_twelve(raw: &[u8]) -> Result<(), BackupError> {
        let text = std::str::from_utf8(raw).map_err(|_| invalid())?;
        let topology = [
            ("/var/lib/postgresql", true),
            ("/var/lib/knowweave-source", true),
            ("/var/lib/knowweave-source/assets", false),
            ("/var/lib/knowweave-c4/registry", true),
            ("/var/lib/knowweave-c4/snapshot-effects-v2", true),
            ("/var/lib/knowweave-c4/snapshot-export-v2", true),
            ("/run/kwc4/sfx", true),
            ("/var/lib/knowweave-c4/b1-source-observation", false),
            ("/opt/knowweave-c4/b1-capture", false),
            ("/usr/bin/docker", false),
            ("/var/run/docker.sock", false),
            ("/var/run/postgresql", true),
        ];
        let mut counts = [0usize; 12];
        for line in text.lines() {
            let mut fields = line.split_ascii_whitespace();
            let target = fields.nth(4).ok_or_else(invalid)?;
            let options = fields.next().ok_or_else(invalid)?;
            if fields.clone().count() < 4 {
                return Err(invalid());
            }
            if let Some((index, (_, rw))) = topology
                .iter()
                .enumerate()
                .find(|(_, (path, _))| *path == target)
            {
                counts[index] += 1;
                if options.split(',').any(|o| o == "rw") != *rw
                    || options.split(',').any(|o| o == "ro") == *rw
                {
                    return Err(invalid());
                }
            } else if topology.iter().any(|(root, _)| {
                target
                    .strip_prefix(root)
                    .is_some_and(|s| s.starts_with('/'))
            }) {
                // The assets child is the sole allowed parent/child member.
                // Any other overlay under the twelve fixed targets rejects.
                return Err(invalid());
            }
        }
        if counts != [1; 12] {
            return Err(invalid());
        }
        Ok(())
    }
    pub(super) fn open(
        roots: &BTreeMap<String, RootRecord>,
        budget: &Budget,
        deadline: Instant,
    ) -> Result<(Self, std::fs::File, std::fs::File), BackupError> {
        use std::os::unix::fs::OpenOptionsExt;
        if Instant::now() >= deadline {
            return Err(invalid());
        }
        let raw = Self::observe_mountinfo(budget)?;
        budget
            .lock()
            .map_err(|_| invalid())?
            .charge((raw.len() as u64) * 3)?;
        Self::require_twelve(&raw)?;
        assets_readonly(
            std::str::from_utf8(&raw).map_err(|_| invalid())?,
            &roots["assets"].path,
        )?;
        let mut held = Vec::new();
        for record in roots.values() {
            budget
                .lock()
                .map_err(|_| invalid())?
                .entries(std::path::Path::new(&record.path).components().count() * 4 + 6)?;
            let path = std::path::PathBuf::from(&record.path);
            let root = learning_assets::backup_fs::BackupDir::open_trusted_private_root(&path)?;
            crate::registry::validate_root_identity(record, &path, root.identity()?)?;
            held.push((path, root, (record.dev, record.ino)));
        }
        budget.lock().map_err(|_| invalid())?.entries(24)?;
        let open = |path: &str| {
            std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(path)
        };
        let companion = open(learning_assets::InstalledSnapshotBinding::COMPANION_PATH)?;
        let export = open(learning_assets::InstalledSnapshotBinding::EXPORT_PATH)?;
        let a = companion.metadata()?;
        let b = export.metadata()?;
        let companion_named = learning_assets::backup_fs::BackupDir::open_trusted_private_root(
            std::path::Path::new(learning_assets::InstalledSnapshotBinding::COMPANION_PATH),
        )?;
        let export_named = learning_assets::backup_fs::BackupDir::open_trusted_private_root(
            std::path::Path::new(learning_assets::InstalledSnapshotBinding::EXPORT_PATH),
        )?;
        if !a.is_dir()
            || !b.is_dir()
            || a.uid() != 0
            || b.uid() != 0
            || a.mode() & 0o7777 != 0o700
            || b.mode() & 0o7777 != 0o700
            || (a.dev(), a.ino()) == (b.dev(), b.ino())
            || companion_named.identity()? != (a.dev(), a.ino())
            || export_named.identity()? != (b.dev(), b.ino())
        {
            return Err(invalid());
        }
        // The normal OS binding constructor still independently checks empty
        // and distinct actual original FDs before making its actual journal.
        let result = Self {
            mountinfo: raw,
            roots: held,
            companion,
            export,
            companion_identity: (a.dev(), a.ino()),
            export_identity: (b.dev(), b.ino()),
            deadline: Some(deadline),
        };
        let (companion, export) = (result.companion.try_clone()?, result.export.try_clone()?);
        Ok((result, companion, export))
    }
    pub(super) fn recheck(&self, budget: &Budget) -> Result<(), BackupError> {
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(invalid());
        }
        let raw = Self::observe_mountinfo(budget)?;
        budget
            .lock()
            .map_err(|_| invalid())?
            .charge((raw.len() + self.mountinfo.len()) as u64)?;
        if raw != self.mountinfo {
            return Err(invalid());
        }
        Self::require_twelve(&raw)?;
        for (path, held, expected) in &self.roots {
            budget
                .lock()
                .map_err(|_| invalid())?
                .entries(path.components().count() * 4 + 6)?;
            if held.identity()? != *expected
                || learning_assets::backup_fs::BackupDir::open_trusted_private_root(path)?
                    .identity()?
                    != *expected
            {
                return Err(invalid());
            }
        }
        budget.lock().map_err(|_| invalid())?.entries(8)?;
        let a = self.companion.metadata()?;
        let b = self.export.metadata()?;
        let named_companion = learning_assets::backup_fs::BackupDir::open_trusted_private_root(
            std::path::Path::new(learning_assets::InstalledSnapshotBinding::COMPANION_PATH),
        )?;
        let named_export = learning_assets::backup_fs::BackupDir::open_trusted_private_root(
            std::path::Path::new(learning_assets::InstalledSnapshotBinding::EXPORT_PATH),
        )?;
        if (a.dev(), a.ino()) != self.companion_identity
            || (b.dev(), b.ino()) != self.export_identity
            || named_companion.identity()? != self.companion_identity
            || named_export.identity()? != self.export_identity
            || !a.is_dir()
            || !b.is_dir()
            || a.uid() != 0
            || b.uid() != 0
            || a.mode() & 0o7777 != 0o700
            || b.mode() & 0o7777 != 0o700
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub(super) fn finish_install(&mut self, budget: &Budget) -> Result<(), BackupError> {
        self.recheck(budget)?;
        // The original120s gate ends at actual installation completion. No
        // new epoch freshness clock or TTL is minted for later work/capture.
        self.deadline = None;
        Ok(())
    }
}

// The executable is management installation input, not stored journal bytes.
// This counter is local to this one pinned-file verification and never exposed
// as an authority, caller-selected allowance or a reusable/resettable meter.
struct CliVerificationBudget {
    used: u64,
    exhausted: bool,
}
impl CliVerificationBudget {
    fn new() -> Self {
        Self {
            used: 0,
            exhausted: false,
        }
    }
    fn pay(&mut self, bytes: u64) -> Result<(), BackupError> {
        if self.exhausted {
            return Err(invalid());
        }
        match self
            .used
            .checked_add(bytes)
            .filter(|n| *n <= 128 * 1024 * 1024)
        {
            Some(next) => {
                self.used = next;
                Ok(())
            }
            None => {
                self.exhausted = true;
                Err(invalid())
            }
        }
    }
}

struct Docker {
    file: std::fs::File,
    identity: (u64, u64, u64),
    budget: Budget,
    journal: SharedJournalBudget,
    deadline: Instant,
}
impl Docker {
    fn open(
        budget: &Budget,
        journal: &SharedJournalBudget,
        deadline: Instant,
    ) -> Result<Self, BackupError> {
        for path in ["/", "/usr", "/usr/bin", DOCKER] {
            let m = std::fs::symlink_metadata(path)?;
            if m.uid() != 0
                || m.mode() & 0o022 != 0
                || m.file_type().is_symlink()
                || (path != DOCKER && !m.is_dir())
                || (path == DOCKER
                    && (!m.is_file() || m.nlink() != 1 || m.len() > 64 * 1024 * 1024))
            {
                return Err(invalid());
            }
        }
        let mut file = std::fs::File::open(DOCKER)?;
        let m = file.metadata()?;
        // Keep the actual read/hash cost on the original management scan
        // budget. The epoch meter pays only the fixed binding description;
        // executable payload is not a journal-prefix or namespace scan.
        charge(budget, journal, 512, 1)?;
        let work = m.len().checked_mul(2).ok_or(BackupError::Overflow)?;
        if let Err(error) = budget.lock().map_err(|_| invalid())?.charge(work) {
            let _ = journal.charge(u64::MAX, u64::MAX);
            return Err(error);
        }
        let mut cli_budget = CliVerificationBudget::new();
        let mut hash = Sha256::new();
        let mut remaining = m.len();
        let mut buffer = [0u8; 65536];
        while remaining > 0 {
            if Instant::now() >= deadline {
                return Err(invalid());
            }
            let count = remaining.min(buffer.len() as u64) as usize;
            cli_budget.pay((count as u64) * 2)?;
            file.read_exact(&mut buffer[..count])?;
            hash.update(&buffer[..count]);
            remaining -= count as u64;
        }
        let mut extra = [0];
        let digest = hex::encode(hash.finalize());
        let after = file.metadata()?;
        if file.read(&mut extra)? != 0
            || (
                m.dev(),
                m.ino(),
                m.len(),
                m.mtime(),
                m.mtime_nsec(),
                m.ctime(),
                m.ctime_nsec(),
            ) != (
                after.dev(),
                after.ino(),
                after.len(),
                after.mtime(),
                after.mtime_nsec(),
                after.ctime(),
                after.ctime_nsec(),
            )
            || option_env!("KNOWWEAVE_C4_DOCKER_CLI_SHA256")
                .is_none_or(|pin| !crate::valid_digest(pin) || pin != digest)
        {
            return Err(invalid());
        }
        Ok(Self {
            file,
            identity: (m.dev(), m.ino(), m.len()),
            budget: budget.clone(),
            journal: journal.clone(),
            deadline,
        })
    }
    async fn call(&self, args: &[&str]) -> Result<Vec<u8>, BackupError> {
        if Instant::now() >= self.deadline {
            return Err(invalid());
        }
        let bytes = args.iter().try_fold(0u64, |n, a| {
            n.checked_add(a.len() as u64).ok_or(BackupError::Overflow)
        })?;
        charge(&self.budget, &self.journal, bytes * 2 + 1024, 1)?;
        let owned: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        let owner = Self {
            file: self.file.try_clone()?,
            identity: self.identity,
            budget: self.budget.clone(),
            journal: self.journal.clone(),
            deadline: self.deadline,
        };
        // Once started, this original owner remains alive even if the caller
        // drops its await. It drains both pipes and waits/reaps its own child.
        tokio::spawn(async move { owner.call_inner(&owned).await })
            .await
            .map_err(|_| invalid())?
    }
    async fn call_inner(&self, args: &[String]) -> Result<Vec<u8>, BackupError> {
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(invalid)?;
        let held = self.file.metadata()?;
        let current = std::fs::symlink_metadata(DOCKER)?;
        for m in [&held, &current] {
            if (m.dev(), m.ino(), m.len()) != self.identity
                || !m.is_file()
                || m.uid() != 0
                || m.mode() & 0o022 != 0
            {
                return Err(invalid());
            }
        }
        let maximum = if args.iter().any(|a| a == "inspect") {
            32768u64
        } else if args.iter().any(|a| a == "ls") {
            131072
        } else {
            4096
        };
        charge(&self.budget, &self.journal, maximum * 2 + 4096, 1)?;
        let mut child = tokio::process::Command::new(DOCKER)
            .args(args)
            .env_clear()
            .env("LC_ALL", "C")
            .env("DOCKER_HOST", "unix:///var/run/docker.sock")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let mut stdout = child.stdout.take().ok_or_else(invalid)?.take(maximum + 1);
        let mut stderr = child.stderr.take().ok_or_else(invalid)?.take(4097);
        let mut out = Vec::new();
        let mut err = Vec::new();
        let result = tokio::time::timeout(remaining.min(Duration::from_secs(20)), async {
            let (a, b, c) = tokio::join!(
                stdout.read_to_end(&mut out),
                stderr.read_to_end(&mut err),
                child.wait()
            );
            a?;
            b?;
            let status = c?;
            Ok::<_, std::io::Error>(status)
        })
        .await;
        let status = match result {
            Ok(Ok(status)) => status,
            _ => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(invalid());
            }
        };
        if Instant::now() >= self.deadline
            || !status.success()
            || !err.is_empty()
            || out.len() as u64 > maximum
        {
            return Err(invalid());
        }
        Ok(out)
    }
    async fn ids(&self) -> Result<Vec<String>, BackupError> {
        let raw = self
            .call(&[
                "container",
                "ls",
                "--all",
                "--no-trunc",
                "--format",
                "{{.ID}}",
            ])
            .await?;
        let text = std::str::from_utf8(&raw).map_err(|_| invalid())?;
        let mut ids: Vec<String> = text.lines().map(str::trim).map(str::to_owned).collect();
        if ids.len() > MAX_CONTAINERS || ids.iter().any(|id| !crate::valid_digest(id)) {
            return Err(invalid());
        }
        ids.sort();
        if ids.windows(2).any(|w| w[0] == w[1]) {
            return Err(invalid());
        }
        Ok(ids)
    }
    async fn inspect(&self, id: &str) -> Result<Value, BackupError> {
        if !crate::valid_digest(id) {
            return Err(invalid());
        }
        let raw = self
            .call(&["container", "inspect", "--format", INSPECT, id])
            .await?;
        charge(
            &self.budget,
            &self.journal,
            (raw.len() as u64)
                .checked_mul(16)
                .ok_or(BackupError::Overflow)?,
            0,
        )?;
        let row: Value = serde_json::from_slice(&raw)?;
        if row["Id"] != id {
            return Err(invalid());
        }
        Ok(row)
    }
}

fn contains(parent: &str, child: &str) -> bool {
    parent == child
        || child
            .strip_prefix(parent)
            .is_some_and(|tail| tail.starts_with('/'))
}
fn mapped(mounts: &[Value], inside: &str) -> Result<String, BackupError> {
    let mount = mounts
        .iter()
        .filter(|m| {
            m["Destination"]
                .as_str()
                .is_some_and(|destination| contains(destination, inside))
        })
        .max_by_key(|m| m["Destination"].as_str().map_or(0, str::len))
        .ok_or_else(invalid)?;
    let source = mount["Source"].as_str().ok_or_else(invalid)?;
    let destination = mount["Destination"].as_str().ok_or_else(invalid)?;
    if !crate::registry::canonical_path(source) {
        return Err(invalid());
    }
    Ok(format!(
        "{}{}",
        source,
        inside.strip_prefix(destination).ok_or_else(invalid)?
    ))
}
fn mount_projection(item: &Value) -> Result<Value, BackupError> {
    let mounts = item["Mounts"].as_array().ok_or_else(invalid)?;
    let mut projection = Vec::new();
    for mount in mounts {
        let source = mount["Source"].as_str().ok_or_else(invalid)?;
        let destination = mount["Destination"].as_str().ok_or_else(invalid)?;
        if !crate::registry::canonical_path(source)
            || !crate::registry::canonical_path(destination)
            || !mount["RW"].is_boolean()
        {
            return Err(invalid());
        }
        projection.push(json!({"source":source,"destination":destination,"rw":mount["RW"],"kind":mount["Type"],"name":mount["Name"]}));
    }
    projection.sort_by(|a, b| a["destination"].as_str().cmp(&b["destination"].as_str()));
    Ok(
        json!({"container_id":item["Id"],"image_sha256":item["Image"],"running":item["State"]["Running"],
        "process_id":item["State"]["Pid"],"started_at":item["State"]["StartedAt"],"restart_count":item["RestartCount"],
        "project":item["Config"]["Labels"]["com.docker.compose.project"],
        "service":item["Config"]["Labels"]["com.docker.compose.service"],"mounts":projection}),
    )
}

fn own_container(mountinfo: &str) -> Result<String, BackupError> {
    let mut ids = std::collections::BTreeSet::new();
    for line in mountinfo.lines().filter(|line| {
        line.contains(" /etc/hostname ")
            || line.contains(" /etc/hosts ")
            || line.contains(" /etc/resolv.conf ")
    }) {
        for token in line.split('/') {
            if crate::valid_digest(token) {
                ids.insert(token.to_owned());
            }
        }
    }
    if ids.len() != 1 {
        return Err(invalid());
    }
    ids.into_iter().next().ok_or_else(invalid)
}

fn assets_readonly(mountinfo: &str, path: &str) -> Result<(), BackupError> {
    let mut found = None;
    for line in mountinfo.lines() {
        let fields: Vec<_> = line.split(' ').collect();
        if fields.len() < 10 {
            return Err(invalid());
        }
        let mount = fields[4];
        if mount.contains('\\') {
            continue;
        }
        if contains(mount, path)
            && found
                .as_ref()
                .is_none_or(|(length, _)| *length < mount.len())
        {
            let split = fields
                .iter()
                .position(|field| *field == "-")
                .ok_or_else(invalid)?;
            let super_options = fields.get(split + 3).ok_or_else(invalid)?;
            found = Some((
                mount.len(),
                fields[5].split(',').any(|opt| opt == "ro")
                    || super_options.split(',').any(|opt| opt == "ro"),
            ));
        }
    }
    if found.is_none_or(|(_, ro)| !ro) {
        return Err(invalid());
    }
    Ok(())
}

/// A private first observation, retaining the actual pinned Docker executable,
/// complete old set/projection and the exact mount namespace until installation
/// has finished. No caller-provided boolean can construct it.
pub(super) struct FreshMounts {
    docker: Docker,
    mountinfo: String,
    own: Value,
    ids: Vec<String>,
    old: Vec<Value>,
    daemon: Vec<u8>,
    export: String,
    companion: String,
}
impl FreshMounts {
    pub(super) async fn open(
        config: &SourceBackupConfig,
        roots: &BTreeMap<String, RootRecord>,
        budget: &Budget,
        journal: &SharedJournalBudget,
        deadline: Instant,
    ) -> Result<(Self, std::fs::File, std::fs::File), BackupError> {
        use std::os::unix::fs::OpenOptionsExt;
        charge(budget, journal, 4 * 1024 * 1024, 2)?;
        let mut mountinfo = String::new();
        std::fs::File::open("/proc/self/mountinfo")?
            .take(1024 * 1024 + 1)
            .read_to_string(&mut mountinfo)?;
        if mountinfo.len() > 1024 * 1024 {
            return Err(invalid());
        }
        assets_readonly(&mountinfo, &roots["assets"].path)?;
        let self_id = own_container(&mountinfo)?;
        let docker = Docker::open(budget, journal, deadline)?;
        let daemon = docker.call(&["info", "--format", "{{.ID}}"]).await?;
        let ids = docker.ids().await?;
        if !ids.contains(&self_id) {
            return Err(invalid());
        }
        let own = docker.inspect(&self_id).await?;
        if own["State"]["Running"] != true
            || !own["Image"]
                .as_str()
                .is_some_and(|s| s.strip_prefix("sha256:").is_some_and(crate::valid_digest))
        {
            return Err(invalid());
        }
        let own_mounts = own["Mounts"].as_array().ok_or_else(invalid)?;
        let dedicated = |path: &str| -> Result<(&Value, String), BackupError> {
            let mut matching = own_mounts.iter().filter(|m| m["Destination"] == path);
            let mount = matching.next().ok_or_else(invalid)?;
            if matching.next().is_some()
                || mount["Type"] != "volume"
                || mount["RW"] != true
                || mount["Name"].as_str().is_none_or(str::is_empty)
                || own_mounts.iter().any(|m| {
                    m["Destination"]
                        .as_str()
                        .is_some_and(|d| d != path && contains(path, d))
                })
            {
                return Err(invalid());
            }
            Ok((mount, mapped(own_mounts, path)?))
        };
        let (export_mount, export) =
            dedicated(learning_assets::InstalledSnapshotBinding::EXPORT_PATH)?;
        let (companion_mount, companion) =
            dedicated(learning_assets::InstalledSnapshotBinding::COMPANION_PATH)?;
        if export_mount["Name"] == companion_mount["Name"]
            || contains(&export, &companion)
            || contains(&companion, &export)
        {
            return Err(invalid());
        }
        let assets_mount = own_mounts
            .iter()
            .filter(|m| {
                m["Destination"]
                    .as_str()
                    .is_some_and(|d| contains(d, &roots["assets"].path))
            })
            .max_by_key(|m| m["Destination"].as_str().map_or(0, str::len))
            .ok_or_else(invalid)?;
        if assets_mount["RW"] != false {
            return Err(invalid());
        }
        let mut old = Vec::new();
        for id in &ids {
            let row = if id == &self_id {
                own.clone()
            } else {
                docker.inspect(id).await?
            };
            if id != &self_id {
                for mount in row["Mounts"].as_array().ok_or_else(invalid)? {
                    let source = mount["Source"].as_str().ok_or_else(invalid)?;
                    if mount["RW"] == true
                        && [export.as_str(), companion.as_str()]
                            .iter()
                            .any(|r| contains(source, r) || contains(r, source))
                    {
                        return Err(invalid());
                    }
                }
                if row["Config"]["Labels"]["com.docker.compose.project"]
                    == config.expected_compose_project
                    && row["Config"]["Labels"]["com.docker.compose.service"] != "pg"
                    && row["State"]["Running"] == true
                {
                    return Err(invalid());
                }
            }
            old.push(mount_projection(&row)?);
        }
        let open = |path| {
            std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(path)
        };
        // The OS constructor independently proves fixed-path, private-empty,
        // distinct directory identities while consuming these actual FDs.
        let companion_fd = open(learning_assets::InstalledSnapshotBinding::COMPANION_PATH)?;
        let export_fd = open(learning_assets::InstalledSnapshotBinding::EXPORT_PATH)?;
        if Instant::now() >= deadline {
            return Err(invalid());
        }
        Ok((
            Self {
                docker,
                mountinfo,
                own,
                ids,
                old,
                daemon,
                export,
                companion,
            },
            companion_fd,
            export_fd,
        ))
    }
    pub(super) async fn finish(
        self,
        config: &SourceBackupConfig,
        descriptor: &Descriptor,
        roots: &BTreeMap<String, RootRecord>,
    ) -> Result<Value, BackupError> {
        if self.docker.ids().await? != self.ids
            || self.docker.call(&["info", "--format", "{{.ID}}"]).await? != self.daemon
        {
            return Err(invalid());
        }
        for (id, expected) in self.ids.iter().zip(&self.old) {
            if mount_projection(&self.docker.inspect(id).await?)? != *expected {
                return Err(invalid());
            }
        }
        let mut after = String::new();
        std::fs::File::open("/proc/self/mountinfo")?
            .take(1024 * 1024 + 1)
            .read_to_string(&mut after)?;
        if after != self.mountinfo || Instant::now() >= self.docker.deadline {
            return Err(invalid());
        }
        let self_id = self.own["Id"].as_str().ok_or_else(invalid)?;
        let assets_mount = self.own["Mounts"]
            .as_array()
            .ok_or_else(invalid)?
            .iter()
            .filter(|m| {
                m["Destination"]
                    .as_str()
                    .is_some_and(|d| contains(d, &roots["assets"].path))
            })
            .max_by_key(|m| m["Destination"].as_str().map_or(0, str::len))
            .ok_or_else(invalid)?;
        let legacy: Vec<_> = self
            .old
            .iter()
            .filter(|r| {
                r["project"] == config.expected_compose_project && r["container_id"] != self_id
            })
            .collect();
        Ok(
            json!({"container_id":self_id,"image_sha256":self.own["Image"],"process_id":descriptor.broker_pid,
            "uid":descriptor.broker_uid,"binary_sha256":descriptor.broker_binary_sha256,
            "application_commit":descriptor.source_application_commit,"build_sha256":descriptor.source_build_sha256,
            "export_mount":self.export,"journal_mount":self.companion,"assets_mount":assets_mount,
            "legacy_producers":legacy,"complete_container_projection":self.old,
            "daemon_id":std::str::from_utf8(&self.daemon).map_err(|_|invalid())?.trim(),
            "mountinfo_sha256":hash(self.mountinfo.as_bytes()),"docker_cli_sha256":option_env!("KNOWWEAVE_C4_DOCKER_CLI_SHA256")}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cutover_requires_actual_ro_cover_and_rejects_longer_rw_overlay() {
        let ro = "1 0 0:1 / / rw - overlay overlay rw\n2 1 0:2 / /assets ro - ext4 /dev/x rw\n";
        assert!(assets_readonly(ro, "/assets/objects").is_ok());
        assert!(assets_readonly(ro, "/other").is_err());
        let overlay = format!("{ro}3 2 0:3 / /assets/objects rw - ext4 /dev/y rw\n");
        assert!(assets_readonly(&overlay, "/assets/objects").is_err());
    }
    #[test]
    fn cutover_host_mapping_uses_longest_actual_mount() {
        let mounts = vec![
            json!({"Source":"/host/base","Destination":"/root","RW":true}),
            json!({"Source":"/host/exact","Destination":"/root/assets","RW":false}),
        ];
        assert_eq!(
            mapped(&mounts, "/root/assets/objects").unwrap(),
            "/host/exact/objects"
        );
        assert!(mapped(&mounts, "/outside").is_err());
        assert!(!contains("/root/a", "/root/ab"));
    }
    #[test]
    fn cli_binding_has_a_separate_fixed_bound_and_sticky_exhaustion() {
        let mut actual = CliVerificationBudget::new();
        // The measured production CLI fits without changing any epoch limit.
        actual.pay(47_222_173 * 2).unwrap();
        assert!(actual.pay(128 * 1024 * 1024).is_err());
        assert!(actual.pay(0).is_err());
        let mut boundary = CliVerificationBudget::new();
        boundary.pay(128 * 1024 * 1024).unwrap();
        assert!(boundary.pay(1).is_err());
        assert!(boundary.pay(0).is_err());
    }
}

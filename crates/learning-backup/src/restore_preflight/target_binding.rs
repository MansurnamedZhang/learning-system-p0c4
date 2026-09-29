//! Root-only Docker/PG observation guard. Retains creation/target locks and
//! exact observed identity, but does not prove the SQLx pool or a host
//! pg_restore connection reaches that Docker endpoint. Never write authority.
use super::*;
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::Value;

const PINNED_IMAGE: &str = "postgres:18.6-bookworm@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d";
const PG_ID_SQL: &str = "SELECT d.oid::bigint::text || '|' || pcs.system_identifier::text FROM pg_catalog.pg_database d CROSS JOIN pg_catalog.pg_control_system() pcs WHERE d.datname=pg_catalog.current_database()";

#[derive(Debug, Clone)]
struct DockerClaim {
    container_id: String,
    network_id: String,
    image_id: String,
    volume_name: String,
    mountpoint: String,
    project: String,
    network_name: String,
    database: String,
    daemon_id: String,
    system_identifier: String,
    database_oid: u64,
    subnet: String,
    target_path: String,
    initdb_source: String,
    mount_dev: u64,
    mount_ino: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PinPrecreation {
    format_version: u32,
    state: String,
    target_absent_at_precreation: bool,
    batch_id: String,
    project: String,
    database: String,
    network: String,
    volume: String,
    subnet: String,
    root_path: String,
    root_dev: u64,
    root_ino: u64,
    targets_dev: u64,
    targets_ino: u64,
    docker_daemon_id: String,
    initdb_path: String,
    initdb_sha256: String,
}

fn parse_precreation(bytes: &[u8]) -> Result<PinPrecreation, BackupError> {
    if bytes.is_empty() || bytes.len() > 4096 {
        return Err(BackupError::Invalid("bound precreation size"));
    }
    let value: Value = serde_json::from_slice(bytes)?;
    if serde_json::to_vec(&value)? != bytes {
        return Err(BackupError::Invalid("noncanonical bound precreation"));
    }
    serde_json::from_value(value).map_err(Into::into)
}

fn validate_precreation(
    record: &PinPrecreation,
    claim: &DockerClaim,
    root: &Path,
    root_id: (u64, u64),
    targets_id: (u64, u64),
    root_entries: &[String],
    targets_entries: &[String],
) -> Result<(), BackupError> {
    let birth_batch = claim
        .database
        .strip_prefix("learning_restore_c4_")
        .ok_or(BackupError::Invalid("bound target batch"))?;
    let parts: Vec<_> = root.components().collect();
    let is_acceptance = parts
        .windows(2)
        .any(|pair| pair[0].as_os_str() == "birth-acceptance" && pair[1].as_os_str() == "batches");
    let mut root_names = root_entries.to_vec();
    root_names.sort();
    if is_acceptance
        || root_names != [".restore-target.lock", "pin-precreation.json", "targets"]
        || targets_entries != [birth_batch]
        || record.format_version != 1
        || record.state != "PIN_ONLY_PRECREATION_NOT_RESTORE_AUTHORITY"
        || !record.target_absent_at_precreation
        || record.batch_id != birth_batch
        || record.project != claim.project
        || record.database != claim.database
        || record.network != claim.network_name
        || record.volume != claim.volume_name
        || record.subnet != claim.subnet
        || record.root_path != root.to_string_lossy()
        || (record.root_dev, record.root_ino) != root_id
        || (record.targets_dev, record.targets_ino) != targets_id
        || record.docker_daemon_id != claim.daemon_id
        || record.initdb_path != claim.initdb_source
        || !crate::valid_digest(&record.initdb_sha256)
    {
        return Err(BackupError::Invalid("bound target precreation differs"));
    }
    Ok(())
}

fn exact_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

fn pg_exec_args(claim: &DockerClaim) -> Result<Vec<String>, BackupError> {
    if !exact_id(&claim.container_id) || !claim.database.starts_with("learning_restore_c4_") {
        return Err(BackupError::Invalid("bound target identity invalid"));
    }
    Ok([
        "exec",
        "-i",
        "--user",
        "postgres",
        &claim.container_id,
        "psql",
        "-XAt",
        "-h",
        "/var/run/postgresql",
        "-U",
        "postgres",
        "-d",
        &claim.database,
        "-v",
        "ON_ERROR_STOP=1",
        "-c",
        PG_ID_SQL,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect())
}

fn validate_pg_line(claim: &DockerClaim, output: &str) -> Result<(), BackupError> {
    let one = output
        .strip_suffix('\n')
        .ok_or(BackupError::Invalid("bound PG result shape"))?;
    let (oid, system) = one
        .split_once('|')
        .ok_or(BackupError::Invalid("bound PG result shape"))?;
    if one.contains('\n')
        || oid.parse::<u64>().ok() != Some(claim.database_oid)
        || system != claim.system_identifier
    {
        return Err(BackupError::Invalid("bound PG identity differs"));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ChallengeKeys([i64; 2]);

impl ChallengeKeys {
    pub(super) fn random() -> Result<Self, BackupError> {
        let rng = SystemRandom::new();
        loop {
            let mut bytes = [0; 16];
            rng.fill(&mut bytes)
                .map_err(|_| BackupError::Invalid("SQL session challenge randomness failed"))?;
            let keys = Self([
                i64::from_be_bytes(bytes[..8].try_into().unwrap()),
                i64::from_be_bytes(bytes[8..].try_into().unwrap()),
            ]);
            if keys.validate().is_ok() {
                return Ok(keys);
            }
        }
    }

    #[cfg(test)]
    fn for_test(first: i64, second: i64) -> Self {
        Self([first, second])
    }

    fn validate(self) -> Result<(), BackupError> {
        if self.0[0] == self.0[1] {
            return Err(BackupError::Invalid("SQL session challenge keys repeat"));
        }
        Ok(())
    }

    pub(super) fn values(self) -> [i64; 2] {
        self.0
    }
}

// The lease is the caller's open SQL transaction. Consuming verification
// drops it on failure; successful verification returns it for continuation.
#[derive(Debug)]
pub(super) struct LockChallenge<L> {
    keys: ChallengeKeys,
    lease: L,
}

impl<L> LockChallenge<L> {
    pub(super) fn new(keys: ChallengeKeys, lease: L) -> Self {
        Self { keys, lease }
    }

    pub(super) fn keys(&self) -> ChallengeKeys {
        self.keys
    }

    pub(super) fn lease_mut(&mut self) -> &mut L {
        &mut self.lease
    }
}

fn lock_halves(key: i64) -> (u32, u32) {
    let bits = key as u64;
    ((bits >> 32) as u32, bits as u32)
}

fn challenge_exec_args(
    claim: &DockerClaim,
    keys: ChallengeKeys,
) -> Result<Vec<String>, BackupError> {
    keys.validate()?;
    let (high_a, low_a) = lock_halves(keys.0[0]);
    let (high_b, low_b) = lock_halves(keys.0[1]);
    let mut args = pg_exec_args(claim)?;
    // Numeric key halves are the only variable SQL. Query all matching locks,
    // including wrong PID/database/mode, so the parser can reject ambiguity.
    *args.last_mut().unwrap() = format!(
        "SELECT pid, database::bigint, classid::bigint, objid::bigint, objsubid, mode, granted \
         FROM pg_catalog.pg_locks WHERE locktype='advisory' \
         AND (classid::bigint,objid::bigint) IN (({high_a},{low_a}),({high_b},{low_b}))"
    );
    Ok(args)
}

fn validate_challenge_rows(
    keys: ChallengeKeys,
    expected_pid: i32,
    expected_database_oid: u64,
    output: &str,
) -> Result<(), BackupError> {
    keys.validate()?;
    if expected_pid <= 0 || expected_database_oid == 0 || output.len() > 4096 {
        return Err(BackupError::Invalid(
            "SQL session challenge expectation invalid",
        ));
    }
    let lines: Vec<_> = output.lines().collect();
    if lines.len() != 2 || !output.ends_with('\n') || output.contains('\r') {
        return Err(BackupError::Invalid("SQL session challenge row count"));
    }
    let mut seen = [false; 2];
    for line in lines {
        let fields: Vec<_> = line.split('|').collect();
        if fields.len() != 7
            || fields[0] != expected_pid.to_string()
            || fields[1] != expected_database_oid.to_string()
            || fields[4] != "1"
            || fields[5] != "ExclusiveLock"
            || fields[6] != "t"
        {
            return Err(BackupError::Invalid("SQL session challenge lock differs"));
        }
        let pair = keys.0.iter().position(|key| {
            let (high, low) = lock_halves(*key);
            fields[2] == high.to_string() && fields[3] == low.to_string()
        });
        let index = pair.ok_or(BackupError::Invalid("SQL session challenge key differs"))?;
        if seen[index] {
            return Err(BackupError::Invalid("SQL session challenge lock repeated"));
        }
        seen[index] = true;
    }
    if seen != [true, true] {
        return Err(BackupError::Invalid("SQL session challenge lock missing"));
    }
    Ok(())
}

fn same_observation(before: &Value, after: &Value) -> Result<(), BackupError> {
    if before != after {
        return Err(BackupError::Invalid("bound target changed during PG probe"));
    }
    Ok(())
}

fn only_claimed_resource(output: &str, expected: &str) -> Result<(), BackupError> {
    if output.strip_suffix('\n') == Some(expected) {
        Ok(())
    } else {
        Err(BackupError::Invalid(
            "bound project has extra or missing Docker resource",
        ))
    }
}

fn optional_existing_lock<T, F>(
    kind: io::Result<BackupEntryKind>,
    open_existing: F,
) -> Result<Option<T>, BackupError>
where
    F: FnOnce() -> Result<T, BackupError>,
{
    match kind {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Ok(BackupEntryKind::File) => open_existing().map(Some),
        Ok(_) => Err(BackupError::Invalid("bound target lock is not a file")),
        Err(error) => Err(error.into()),
    }
}

fn val<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |row, key| row.get(*key))
}

fn eq_str(value: &Value, path: &[&str], expected: &str) -> bool {
    val(value, path).and_then(Value::as_str) == Some(expected)
}

fn validate_docker(
    claim: &DockerClaim,
    pg: &Value,
    network: &Value,
    volume: &Value,
) -> Result<Value, BackupError> {
    let invalid = BackupError::Invalid("bound Docker identity differs");
    if !exact_id(&claim.container_id)
        || !exact_id(&claim.network_id)
        || !claim.image_id.strip_prefix("sha256:").is_some_and(exact_id)
        || !eq_str(pg, &["Id"], &claim.container_id)
        || !eq_str(pg, &["Image"], &claim.image_id)
        || !eq_str(pg, &["Config", "Image"], PINNED_IMAGE)
        || !eq_str(
            pg,
            &["Config", "Labels", "com.docker.compose.project"],
            &claim.project,
        )
        || !eq_str(
            pg,
            &["Config", "Labels", "com.docker.compose.service"],
            "pg",
        )
        || val(pg, &["State", "Running"]) != Some(&Value::Bool(true))
        || !eq_str(pg, &["State", "Health", "Status"], "healthy")
        || !eq_str(pg, &["HostConfig", "NetworkMode"], &claim.network_name)
        || !eq_str(network, &["Id"], &claim.network_id)
        || !eq_str(network, &["Name"], &claim.network_name)
        || !eq_str(
            network,
            &["Labels", "com.docker.compose.project"],
            &claim.project,
        )
        || val(network, &["Internal"]) != Some(&Value::Bool(true))
        || val(network, &["IPAM", "Config"])
            .and_then(Value::as_array)
            .is_none_or(|configs| {
                configs.len() != 1 || !eq_str(&configs[0], &["Subnet"], &claim.subnet)
            })
        || !eq_str(volume, &["Name"], &claim.volume_name)
        || !eq_str(volume, &["Mountpoint"], &claim.mountpoint)
        || !eq_str(
            volume,
            &["Labels", "com.docker.compose.project"],
            &claim.project,
        )
        || !eq_str(volume, &["Driver"], "local")
        || !eq_str(volume, &["Scope"], "local")
        || !volume.get("Options").is_some_and(|options| {
            options.is_null()
                || options
                    .as_object()
                    .is_some_and(|entries| entries.is_empty())
        })
    {
        return Err(invalid);
    }
    let attached = val(pg, &["NetworkSettings", "Networks"])
        .and_then(Value::as_object)
        .ok_or(BackupError::Invalid("bound network shape"))?;
    if attached.len() != 1
        || attached
            .get(&claim.network_name)
            .and_then(|n| n.get("NetworkID"))
            .and_then(Value::as_str)
            != Some(&claim.network_id)
        || val(pg, &["NetworkSettings", "Ports"])
            .and_then(Value::as_object)
            .is_none_or(|ports| ports.values().any(|v| !v.is_null()))
        || val(pg, &["HostConfig", "PortBindings"])
            .and_then(Value::as_object)
            .is_none_or(|ports| ports.values().any(|v| !v.is_null()))
    {
        return Err(BackupError::Invalid("bound PG network differs"));
    }
    let members = network
        .get("Containers")
        .and_then(Value::as_object)
        .ok_or(BackupError::Invalid("bound network members absent"))?;
    if members.len() != 1 || !members.contains_key(&claim.container_id) {
        return Err(BackupError::Invalid("bound network has another member"));
    }
    let mounts = pg
        .get("Mounts")
        .and_then(Value::as_array)
        .ok_or(BackupError::Invalid("bound PG mount shape"))?;
    let data: Vec<_> = mounts
        .iter()
        .filter(|m| eq_str(m, &["Destination"], "/var/lib/postgresql"))
        .collect();
    if data.len() != 1
        || !eq_str(data[0], &["Type"], "volume")
        || !eq_str(data[0], &["Name"], &claim.volume_name)
        || !eq_str(data[0], &["Source"], &claim.mountpoint)
        || val(data[0], &["RW"]) != Some(&Value::Bool(true))
    {
        return Err(BackupError::Invalid("bound PG volume mount differs"));
    }
    let expected = [
        (
            "/var/lib/postgresql",
            "volume",
            claim.mountpoint.as_str(),
            true,
        ),
        (
            "/docker-entrypoint-initdb.d/10-restore.sh",
            "bind",
            claim.initdb_source.as_str(),
            false,
        ),
        ("/run/secrets/postgres_password", "bind", "", false),
        ("/run/secrets/admin_password", "bind", "", false),
    ];
    if mounts.len() != expected.len()
        || expected.iter().any(|(dest, kind, source, rw)| {
            let matching: Vec<_> = mounts
                .iter()
                .filter(|m| eq_str(m, &["Destination"], dest))
                .collect();
            if matching.len() != 1 {
                return true;
            }
            let expected_source = if source.is_empty() {
                format!(
                    "{}/secrets/{}",
                    claim.target_path,
                    dest.rsplit('/').next().unwrap()
                )
            } else {
                (*source).to_owned()
            };
            !eq_str(matching[0], &["Type"], kind)
                || !eq_str(matching[0], &["Source"], &expected_source)
                || val(matching[0], &["RW"]) != Some(&Value::Bool(*rw))
        })
    {
        return Err(BackupError::Invalid("bound PG mounts differ"));
    }
    let started = val(pg, &["State", "StartedAt"])
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or(BackupError::Invalid("bound PG start time absent"))?;
    let restart = pg
        .get("RestartCount")
        .and_then(Value::as_u64)
        .ok_or(BackupError::Invalid("bound PG restart count absent"))?;
    Ok(
        serde_json::json!({"container_id":claim.container_id,"network_id":claim.network_id,
        "image_id":claim.image_id,"volume_name":claim.volume_name,
        "mountpoint":claim.mountpoint,"started_at":started,"restart_count":restart,
        "mounts":mounts}),
    )
}

// The dependency boundary is generic so lock ordering and ownership can be
// exercised without a privileged Docker daemon.
#[derive(Debug)]
pub(super) struct BoundTargetGuard<G, T> {
    // Drop the target lock before releasing the global creation lock.
    _target_lock: T,
    _global_lock: G,
    claim: DockerClaim,
    observation: Value,
}

fn acquire_bound_guard<G, T>(
    global: impl FnOnce() -> Result<G, BackupError>,
    target: impl FnOnce() -> Result<T, BackupError>,
    identity: impl FnOnce() -> Result<(DockerClaim, Value), BackupError>,
) -> Result<BoundTargetGuard<G, T>, BackupError> {
    let global_lock = global()?;
    let target_lock = target()?;
    let (claim, observation) = identity()?;
    Ok(BoundTargetGuard {
        _target_lock: target_lock,
        _global_lock: global_lock,
        claim,
        observation,
    })
}

impl<G, T> BoundTargetGuard<G, T> {
    fn verify_sql_session_with<L>(
        &self,
        challenge: LockChallenge<L>,
        expected_pid: i32,
        expected_database_oid: u64,
        mut observe: impl FnMut(&DockerClaim) -> Result<Value, BackupError>,
        query: impl FnOnce(&DockerClaim, &[String]) -> Result<String, BackupError>,
    ) -> Result<LockChallenge<L>, BackupError> {
        if expected_database_oid != self.claim.database_oid {
            return Err(BackupError::Invalid(
                "SQL session challenge database differs",
            ));
        }
        let before = observe(&self.claim)?;
        same_observation(&self.observation, &before)?;
        let args = challenge_exec_args(&self.claim, challenge.keys)?;
        let output = query(&self.claim, &args)?;
        validate_challenge_rows(
            challenge.keys,
            expected_pid,
            self.claim.database_oid,
            &output,
        )?;
        let after = observe(&self.claim)?;
        same_observation(&self.observation, &after)?;
        Ok(challenge)
    }

    fn recheck_with(
        &self,
        mut observe: impl FnMut(&DockerClaim) -> Result<Value, BackupError>,
        query: impl FnOnce(&DockerClaim) -> Result<String, BackupError>,
    ) -> Result<(), BackupError> {
        let before = observe(&self.claim)?;
        same_observation(&self.observation, &before)?;
        validate_pg_line(&self.claim, &query(&self.claim)?)?;
        let after = observe(&self.claim)?;
        same_observation(&self.observation, &after)
    }
}

#[cfg(target_os = "linux")]
pub(super) use linux::acquire_for_restore;

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::{
        os::fd::AsRawFd,
        os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        process::{Command, Stdio},
    };

    #[derive(Deserialize)]
    struct BirthEvidence {
        docker_daemon_id: String,
        container_id: String,
        network_id: String,
        image_id: String,
        volume_name: String,
        volume_mountpoint: String,
        birth_sha256: String,
    }

    fn lock_existing(dir: &BackupDir, name: &str) -> Result<File, BackupError> {
        lock_file(dir.open_file(name)?)
    }

    fn lock_file(file: File) -> Result<File, BackupError> {
        let meta = file.metadata()?;
        if meta.uid() != 0
            || !meta.is_file()
            || meta.nlink() != 1
            || meta.permissions().mode() & 0o777 != 0o600
        {
            return Err(BackupError::Invalid("bound target lock unsafe"));
        }
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(BackupError::Invalid("bound target already locked"));
        }
        Ok(file)
    }

    fn docker(args: &[String]) -> Result<String, BackupError> {
        for path in ["/usr", "/usr/bin", "/usr/bin/docker"] {
            let meta = std::fs::symlink_metadata(path)?;
            if meta.uid() != 0
                || meta.permissions().mode() & 0o022 != 0
                || (path.ends_with("docker") && !meta.is_file())
                || (!path.ends_with("docker") && !meta.is_dir())
            {
                return Err(BackupError::Invalid("Docker executable path untrusted"));
            }
        }
        let result = Command::new("/usr/bin/docker")
            .args(args)
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/bin")
            .env("DOCKER_HOST", "unix:///var/run/docker.sock")
            .stdin(Stdio::null())
            .output()?;
        if !result.status.success() || result.stdout.len() > 1024 * 1024 {
            return Err(BackupError::Invalid("bound Docker observation failed"));
        }
        String::from_utf8(result.stdout)
            .map_err(|_| BackupError::Invalid("bound Docker output invalid"))
    }

    #[cfg(test)]
    pub(super) fn assert_guard_pg_empty(claim: &DockerClaim) {
        // Catalog SELECTs only, over the same exact-ID local socket path.
        let mut args = pg_exec_args(claim).unwrap();
        *args.last_mut().unwrap() = "SELECT \
            (SELECT count(*) FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
             WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname NOT LIKE 'pg_toast%' AND n.nspname NOT LIKE 'pg_temp%')::text || '|' || \
            (SELECT count(*) FROM pg_catalog.pg_largeobject_metadata)::text || '|' || \
            (SELECT count(*) FROM pg_catalog.pg_namespace WHERE nspname NOT IN ('pg_catalog','information_schema','public') \
             AND nspname NOT LIKE 'pg_toast%' AND nspname NOT LIKE 'pg_temp%')::text".into();
        assert_eq!(docker(&args).unwrap(), "0|0|0\n");
    }

    fn inspect(kind: &str, id: &str) -> Result<Value, BackupError> {
        let args = [kind.to_owned(), "inspect".into(), id.into()];
        let result: Value = serde_json::from_str(&docker(&args)?)?;
        let rows = result
            .as_array()
            .ok_or(BackupError::Invalid("bound Docker inspect shape"))?;
        if rows.len() != 1 {
            return Err(BackupError::Invalid("bound Docker inspect count"));
        }
        Ok(rows[0].clone())
    }

    fn reviewed_initdb(path: &Path) -> Result<String, BackupError> {
        if !path.is_absolute() {
            return Err(BackupError::Invalid("bound initdb path"));
        }
        for ancestor in path.ancestors() {
            let meta = std::fs::symlink_metadata(ancestor)?;
            if meta.uid() != 0
                || meta.permissions().mode() & 0o022 != 0
                || (ancestor == path
                    && (!meta.is_file()
                        || meta.nlink() != 1
                        || meta.permissions().mode() & 0o777 != 0o444))
                || (ancestor != path && !meta.is_dir())
            {
                return Err(BackupError::Invalid("bound initdb source unsafe"));
            }
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        let opened = file.metadata()?;
        if !opened.is_file()
            || opened.uid() != 0
            || opened.nlink() != 1
            || opened.permissions().mode() & 0o777 != 0o444
            || opened.len() == 0
            || opened.len() > 1024 * 1024
        {
            return Err(BackupError::Invalid("bound initdb changed"));
        }
        let mut bytes = Vec::new();
        file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 != opened.len() {
            return Err(BackupError::Invalid("bound initdb length changed"));
        }
        Ok(format!("{:x}", Sha256::digest(&bytes)))
    }

    fn verify_mount_inode(claim: &DockerClaim) -> Result<(), BackupError> {
        let meta = std::fs::symlink_metadata(&claim.mountpoint)?;
        if !meta.is_dir() || meta.dev() != claim.mount_dev || meta.ino() != claim.mount_ino {
            return Err(BackupError::Invalid("bound PG volume inode differs"));
        }
        Ok(())
    }

    fn observe(claim: &DockerClaim) -> Result<Value, BackupError> {
        verify_mount_inode(claim)?;
        if docker(&["info".into(), "--format".into(), "{{.ID}}".into()])?.trim() != claim.daemon_id
        {
            return Err(BackupError::Invalid("bound Docker daemon differs"));
        }
        let label = format!("label=com.docker.compose.project={}", claim.project);
        for (args, expected) in [
            (
                vec!["container", "ls", "-aq", "--no-trunc", "--filter", &label],
                claim.container_id.as_str(),
            ),
            (
                vec!["network", "ls", "-q", "--no-trunc", "--filter", &label],
                claim.network_id.as_str(),
            ),
            (
                vec!["volume", "ls", "-q", "--filter", &label],
                claim.volume_name.as_str(),
            ),
        ] {
            let args: Vec<String> = args.into_iter().map(str::to_owned).collect();
            only_claimed_resource(&docker(&args)?, expected)?;
        }
        let pg = inspect("container", &claim.container_id)?;
        let network = inspect("network", &claim.network_id)?;
        let volume = inspect("volume", &claim.volume_name)?;
        let image = inspect("image", &claim.image_id)?;
        let image_digest = PINNED_IMAGE.split_once('@').unwrap().1;
        if image.get("Id").and_then(Value::as_str) != Some(&claim.image_id)
            || !image
                .get("RepoDigests")
                .and_then(Value::as_array)
                .is_some_and(|rows| {
                    rows.iter()
                        .any(|d| d.as_str().is_some_and(|s| s.ends_with(image_digest)))
                })
        {
            return Err(BackupError::Invalid("bound PG image digest differs"));
        }
        let projection = validate_docker(claim, &pg, &network, &volume)?;
        verify_mount_inode(claim)?;
        Ok(projection)
    }

    impl BoundTargetGuard<File, Option<File>> {
        pub(in crate::restore_preflight) fn recheck(&self) -> Result<(), BackupError> {
            self.recheck_with(observe, |claim| docker(&pg_exec_args(claim)?))
        }

        pub(in crate::restore_preflight) fn verify_sql_session<L>(
            &self,
            challenge: LockChallenge<L>,
            expected_pid: i32,
            expected_database_oid: u64,
        ) -> Result<LockChallenge<L>, BackupError> {
            self.verify_sql_session_with(
                challenge,
                expected_pid,
                expected_database_oid,
                observe,
                |_, args| docker(args),
            )
        }
    }

    /// Standalone inspection keeps the control directory unchanged.
    pub(super) fn probe_bound_target(config: &RestorePreflightConfig) -> Result<(), BackupError> {
        let _guard = acquire(config, false)?;
        Ok(())
    }

    pub(in crate::restore_preflight) fn acquire_for_restore(
        config: &RestorePreflightConfig,
    ) -> Result<BoundTargetGuard<File, Option<File>>, BackupError> {
        acquire(config, true)
    }

    fn acquire(
        config: &RestorePreflightConfig,
        create_target_lock: bool,
    ) -> Result<BoundTargetGuard<File, Option<File>>, BackupError> {
        if unsafe { libc::geteuid() } != 0 {
            return Err(BackupError::Invalid("root bound target probe required"));
        }
        config.validate()?;
        let target_path = config
            .control_root
            .parent()
            .ok_or(BackupError::Invalid("bound target path"))?;
        let targets = target_path
            .parent()
            .ok_or(BackupError::Invalid("bound targets path"))?;
        if targets.file_name().and_then(|s| s.to_str()) != Some("targets")
            || config.destination_root.parent() != Some(target_path)
            || config.asset_root.parent() != Some(target_path)
        {
            return Err(BackupError::Invalid("bound target roots differ"));
        }
        let root_path = targets
            .parent()
            .ok_or(BackupError::Invalid("bound root path"))?;
        let root = BackupDir::open_trusted_private_root(root_path)?;
        let target = BackupDir::open_trusted_private_root(target_path)?;
        let control = BackupDir::open_trusted_private_root(&config.control_root)?;
        let lock_name = format!("{}.restore.lock", config.expected_database);
        acquire_bound_guard(
            || lock_existing(&root, ".restore-target.lock"),
            || {
                if create_target_lock {
                    let file = match control.create_file(&lock_name) {
                        Ok(file) => {
                            file.sync_all()?;
                            control.sync()?;
                            file
                        }
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                            control.open_file(&lock_name)?
                        }
                        Err(error) => return Err(error.into()),
                    };
                    Ok(Some(lock_file(file)?))
                } else {
                    // Preserve standalone probe's no-create-lock behavior.
                    optional_existing_lock(control.kind(&lock_name), || {
                        lock_existing(&control, &lock_name)
                    })
                }
            },
            || {
                let pinned = option_env!("KNOWWEAVE_C4_TARGET_BIRTH_SHA256").ok_or(
                    BackupError::Invalid("target birth digest is not build-pinned"),
                )?;
                let birth_bytes = read_private_target_file(
                    &control,
                    &format!("{}.birth.json", config.expected_database),
                    4096,
                )?;
                let birth = parse_pinned_birth(&birth_bytes, pinned)?;
                validate_target_dir_batch(target_path, &birth)?;
                let mut failure_present = false;
                for name in ["failure.json", "issuer-diagnostic.json"] {
                    match target.kind(name) {
                        Ok(_) => failure_present = true,
                        Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                        Err(error) => return Err(error.into()),
                    }
                }
                let state_bytes = read_private_target_file(&target, "state.json", 4096)?;
                let success_bytes =
                    read_private_target_file(&target, "issuance-success.json", 4096)?;
                validate_issuance_bytes(
                    &birth,
                    pinned,
                    &state_bytes,
                    Some(&success_bytes),
                    failure_present,
                )?;
                let state: TargetCreationState = serde_json::from_slice(&state_bytes)?;
                let success: TargetIssuanceSuccess = serde_json::from_slice(&success_bytes)?;
                let evidence: BirthEvidence = serde_json::from_slice(&read_private_target_file(
                    &target,
                    "birth-evidence.json",
                    4096,
                )?)?;
                let precreation = parse_precreation(&read_private_target_file(
                    &root,
                    "pin-precreation.json",
                    4096,
                )?)?;
                if evidence.birth_sha256 != pinned
                    || evidence.container_id != success.container_id
                    || evidence.network_id != success.network_id
                    || evidence.image_id != success.image_id
                    || evidence.volume_name != success.pg_volume_name
                    || evidence.volume_mountpoint != success.volume_mountpoint
                {
                    return Err(BackupError::Invalid("bound issuance evidence differs"));
                }
                let claim = DockerClaim {
                    container_id: success.container_id,
                    network_id: success.network_id,
                    image_id: success.image_id,
                    volume_name: success.pg_volume_name,
                    mountpoint: success.volume_mountpoint,
                    project: birth.project_name.clone(),
                    network_name: state.network,
                    database: birth.database_name.clone(),
                    daemon_id: evidence.docker_daemon_id,
                    system_identifier: birth.pg_system_identifier,
                    database_oid: birth.database_oid,
                    subnet: state.subnet,
                    target_path: target_path.to_string_lossy().into_owned(),
                    initdb_source: precreation.initdb_path.clone(),
                    mount_dev: success.volume_mount_dev,
                    mount_ino: success.volume_mount_ino,
                };
                if claim.database != config.expected_database {
                    return Err(BackupError::Invalid("bound database differs"));
                }
                let targets_dir = BackupDir::open_trusted_private_root(targets)?;
                validate_precreation(
                    &precreation,
                    &claim,
                    root_path,
                    root.identity()?,
                    targets_dir.identity()?,
                    &root.list()?,
                    &targets_dir.list()?,
                )?;
                if reviewed_initdb(Path::new(&precreation.initdb_path))?
                    != precreation.initdb_sha256
                {
                    return Err(BackupError::Invalid("bound initdb source differs"));
                }
                let before = observe(&claim)?;
                let sql = docker(&pg_exec_args(&claim)?)?;
                validate_pg_line(&claim, &sql)?;
                let after = observe(&claim)?;
                same_observation(&before, &after)?;
                Ok((claim, before))
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sql_session_challenge_generates_distinct_unpredictable_keys() {
        let first = ChallengeKeys::random().unwrap();
        let second = ChallengeKeys::random().unwrap();
        assert!(first.validate().is_ok());
        assert_ne!(first.values(), second.values());
        assert_eq!(lock_halves(-1), (u32::MAX, u32::MAX));
    }

    #[test]
    fn sql_session_challenge_accepts_exactly_two_live_locks_on_one_backend() {
        let guard = BoundTargetGuard {
            _target_lock: (),
            _global_lock: (),
            claim: claim(),
            observation: json!({"restart_count": 0}),
        };
        let keys = ChallengeKeys::for_test(7, 4_294_967_298);
        let challenge = LockChallenge::new(keys, ());
        let returned = guard
            .verify_sql_session_with(
                challenge,
                123,
                16385,
                |_| Ok(json!({"restart_count": 0})),
                |c, args| {
                    assert_eq!(c.container_id, ID);
                    assert_eq!(&args[..6], ["exec", "-i", "--user", "postgres", ID, "psql"]);
                    assert!(
                        args.windows(2)
                            .any(|pair| pair == ["-h", "/var/run/postgresql"])
                    );
                    assert!(args.last().unwrap().contains("pg_catalog.pg_locks"));
                    assert!(args.last().unwrap().contains("((0,7),(1,2))"));
                    Ok("123|16385|0|7|1|ExclusiveLock|t\n123|16385|1|2|1|ExclusiveLock|t\n".into())
                },
            )
            .unwrap();
        assert_eq!(returned.keys().values(), [7, 4_294_967_298]);
    }

    #[test]
    fn sql_session_challenge_rejects_transaction_database_oid_before_docker_query() {
        use std::{cell::Cell, rc::Rc};
        struct Lease(Rc<Cell<bool>>);
        impl Drop for Lease {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }
        let guard = BoundTargetGuard {
            _target_lock: (),
            _global_lock: (),
            claim: claim(),
            observation: json!({"restart_count": 0}),
        };
        let released = Rc::new(Cell::new(false));
        let challenge = LockChallenge::new(
            ChallengeKeys::for_test(7, 4_294_967_298),
            Lease(released.clone()),
        );
        assert!(
            guard
                .verify_sql_session_with(
                    challenge,
                    123,
                    16386,
                    |_| panic!("wrong database must fail before Docker observation"),
                    |_, _| panic!("wrong database must not query Docker"),
                )
                .is_err()
        );
        assert!(released.get());
    }

    #[test]
    fn sql_session_challenge_rejects_missing_duplicate_foreign_and_malformed_rows() {
        let keys = ChallengeKeys::for_test(7, 4_294_967_298);
        let expected = "123|16385|0|7|1|ExclusiveLock|t\n123|16385|1|2|1|ExclusiveLock|t\n";
        for output in [
            "",
            "123|16385|0|7|1|ExclusiveLock|t\n",
            "123|16385|0|7|1|ExclusiveLock|t\n123|16385|0|7|1|ExclusiveLock|t\n",
            "123|16385|0|7|1|ExclusiveLock|t\n124|16385|1|2|1|ExclusiveLock|t\n",
            "123|16385|0|7|1|ExclusiveLock|t\n123|16386|1|2|1|ExclusiveLock|t\n",
            "123|16385|0|7|1|ExclusiveLock|t\n123|16385|1|3|1|ExclusiveLock|t\n",
            "123|16385|0|7|1|ExclusiveLock|t\n123|16385|1|2|2|ExclusiveLock|t\n",
            "123|16385|0|7|1|ExclusiveLock|t\n123|16385|1|2|1|ShareLock|t\n",
            "123|16385|0|7|1|ExclusiveLock|t\n123|16385|1|2|1|ExclusiveLock|f\n",
            "123|16385|0|7|1|ExclusiveLock|t\n123|16385|1|2|1|ExclusiveLock|t\n125|16385|0|7|1|ExclusiveLock|t\n",
            "123|16385|0|7|1|ExclusiveLock|t\n123|16385|1|2|1|ExclusiveLock|t",
            "123|16385|0|7|1|ExclusiveLock|t|extra\n123|16385|1|2|1|ExclusiveLock|t\n",
        ] {
            assert!(
                validate_challenge_rows(keys, 123, 16385, output).is_err(),
                "accepted {output:?}"
            );
        }
        assert!(validate_challenge_rows(keys, 123, 16385, expected).is_ok());
        assert!(validate_challenge_rows(keys, 0, 16385, expected).is_err());
        assert!(validate_challenge_rows(keys, 123, 0, expected).is_err());
        assert!(ChallengeKeys::for_test(7, 7).validate().is_err());
    }

    #[test]
    fn sql_session_challenge_fails_closed_before_or_after_docker_query_and_releases_lease() {
        use std::{cell::RefCell, rc::Rc};
        struct Lease(Rc<RefCell<Vec<&'static str>>>);
        impl Drop for Lease {
            fn drop(&mut self) {
                self.0.borrow_mut().push("released");
            }
        }
        let guard = BoundTargetGuard {
            _target_lock: (),
            _global_lock: (),
            claim: claim(),
            observation: json!({"restart_count": 0}),
        };
        for drift_after in [false, true] {
            let events = Rc::new(RefCell::new(Vec::new()));
            let mut observations = 0;
            let result = guard.verify_sql_session_with(
                LockChallenge::new(ChallengeKeys::for_test(7, 4_294_967_298), Lease(events.clone())),
                123,
                16385,
                |_| {
                    events.borrow_mut().push("observe");
                    observations += 1;
                    Ok(json!({"restart_count": if observations == 2 || !drift_after { 1 } else { 0 }}))
                },
                |_, _| {
                    events.borrow_mut().push("query");
                    Ok("123|16385|0|7|1|ExclusiveLock|t\n123|16385|1|2|1|ExclusiveLock|t\n".into())
                },
            );
            assert!(result.is_err());
            assert_eq!(
                *events.borrow(),
                if drift_after {
                    ["observe", "query", "observe", "released"].as_slice()
                } else {
                    ["observe", "released"].as_slice()
                }
            );
        }
        let events = Rc::new(RefCell::new(Vec::new()));
        let result = guard.verify_sql_session_with(
            LockChallenge::new(
                ChallengeKeys::for_test(7, 4_294_967_298),
                Lease(events.clone()),
            ),
            123,
            16385,
            |_| {
                events.borrow_mut().push("observe");
                Ok(json!({"restart_count": 0}))
            },
            |_, _| {
                events.borrow_mut().push("query");
                Err(BackupError::Invalid("query failed"))
            },
        );
        assert!(result.is_err());
        assert_eq!(*events.borrow(), ["observe", "query", "released"]);

        let events = Rc::new(RefCell::new(Vec::new()));
        let mut retained = guard
            .verify_sql_session_with(
                LockChallenge::new(
                    ChallengeKeys::for_test(7, 4_294_967_298),
                    Lease(events.clone()),
                ),
                123,
                16385,
                |_| {
                    events.borrow_mut().push("observe");
                    Ok(json!({"restart_count": 0}))
                },
                |_, _| {
                    events.borrow_mut().push("query");
                    Ok("123|16385|0|7|1|ExclusiveLock|t\n123|16385|1|2|1|ExclusiveLock|t\n".into())
                },
            )
            .unwrap();
        assert_eq!(*events.borrow(), ["observe", "query", "observe"]);
        let _lease = retained.lease_mut();
        drop(retained);
        assert_eq!(
            *events.borrow(),
            ["observe", "query", "observe", "released"]
        );
    }

    #[test]
    fn bound_guard_retains_both_locks_until_continuation_is_dropped() {
        use std::cell::RefCell;
        use std::rc::Rc;
        struct Lease(&'static str, Rc<RefCell<Vec<&'static str>>>);
        impl Drop for Lease {
            fn drop(&mut self) {
                self.1.borrow_mut().push(self.0);
            }
        }
        let events = Rc::new(RefCell::new(Vec::new()));
        let guard = acquire_bound_guard(
            || {
                events.borrow_mut().push("global");
                Ok(Lease("drop-global", events.clone()))
            },
            || {
                events.borrow_mut().push("target");
                Ok(Lease("drop-target", events.clone()))
            },
            || {
                events.borrow_mut().push("identity");
                Ok((claim(), json!({"restart_count": 0})))
            },
        )
        .unwrap();
        assert_eq!(*events.borrow(), ["global", "target", "identity"]);
        // Ownership is moved, exactly as from preflight into its continuation.
        let continuation = guard;
        assert_eq!(*events.borrow(), ["global", "target", "identity"]);
        drop(continuation);
        assert_eq!(
            *events.borrow(),
            ["global", "target", "identity", "drop-target", "drop-global"]
        );
    }

    #[test]
    fn bound_guard_global_failure_precedes_all_target_mutation() {
        for reason in ["missing", "unsafe", "busy"] {
            let result = acquire_bound_guard::<(), ()>(
                || Err(BackupError::Invalid(reason)),
                || panic!("target lock or attempt marker must not be created"),
                || panic!("target must not be observed before locks"),
            );
            assert!(matches!(result, Err(BackupError::Invalid(value)) if value == reason));
        }
    }

    #[test]
    fn bound_guard_rechecks_original_identity_before_and_after_pg_query() {
        let guard = BoundTargetGuard {
            _target_lock: (),
            _global_lock: (),
            claim: claim(),
            observation: json!({"restart_count": 0}),
        };
        assert!(
            guard
                .recheck_with(
                    |_| Ok(json!({"restart_count": 1})),
                    |_| panic!("drift before query must fail closed"),
                )
                .is_err()
        );
        assert!(
            guard
                .recheck_with(
                    |_| Ok(json!({"restart_count": 0})),
                    |_| Ok("16386|7361082129910479001\n".into()),
                )
                .is_err()
        );
        let mut observation = 0;
        assert!(
            guard
                .recheck_with(
                    |_| {
                        let value = json!({"restart_count": observation});
                        observation += 1;
                        Ok(value)
                    },
                    |_| Ok("16385|7361082129910479001\n".into()),
                )
                .is_err()
        );
        assert!(
            guard
                .recheck_with(
                    |_| Ok(json!({"restart_count": 0})),
                    |c| {
                        assert_eq!(c.container_id, ID);
                        Ok("16385|7361082129910479001\n".into())
                    },
                )
                .is_ok()
        );
    }

    #[test]
    fn guard_snapshot_rejects_attempt_data_writes_and_missing_or_nonempty_lock() {
        let before = std::collections::BTreeMap::from([(
            "control/birth.json".to_owned(),
            b"sealed".to_vec(),
        )]);
        let lock = "control/test.restore.lock";
        assert!(!guard_files_unchanged(&before, &before, lock));
        let mut after = before.clone();
        after.insert(lock.into(), Vec::new());
        assert!(guard_files_unchanged(&before, &after, lock));
        for path in [
            "control/test.restore.attempt",
            "destination/dump",
            "assets/file",
        ] {
            let mut dirty = after.clone();
            dirty.insert(path.into(), b"write".to_vec());
            assert!(!guard_files_unchanged(&before, &dirty, lock));
        }
        after.insert(lock.into(), b"write".to_vec());
        assert!(!guard_files_unchanged(&before, &after, lock));
        after.insert(lock.into(), Vec::new());
        after.insert("control/birth.json".into(), b"changed".to_vec());
        assert!(!guard_files_unchanged(&before, &after, lock));
    }

    fn guard_files_unchanged(
        before: &std::collections::BTreeMap<String, Vec<u8>>,
        after: &std::collections::BTreeMap<String, Vec<u8>>,
        lock: &str,
    ) -> bool {
        if before.contains_key(lock) {
            return false;
        }
        let mut expected = before.clone();
        expected.insert(lock.into(), Vec::new());
        &expected == after
    }

    fn probe_config_from(
        get: impl Fn(&str) -> Option<String>,
    ) -> Result<RestorePreflightConfig, BackupError> {
        let required = |key| {
            get(key)
                .filter(|value| !value.is_empty())
                .ok_or(BackupError::Invalid("bound probe environment missing"))
        };
        let config = RestorePreflightConfig {
            destination_root: required("KNOWWEAVE_C4_PROBE_DESTINATION_ROOT")?.into(),
            control_root: required("KNOWWEAVE_C4_PROBE_CONTROL_ROOT")?.into(),
            asset_root: required("KNOWWEAVE_C4_PROBE_ASSET_ROOT")?.into(),
            expected_database: required("KNOWWEAVE_C4_PROBE_EXPECTED_DATABASE")?,
            // The probe never opens verifier trust. This absolute value stays
            // outside all target roots and is needed only by config validation.
            trust_path: std::env::temp_dir(),
        };
        config.validate()?;
        Ok(config)
    }

    #[test]
    fn probe_environment_requires_all_explicit_nonsecret_inputs() {
        let target = std::env::temp_dir().join("bound-probe-test-target");
        let destination = target.join("database").to_string_lossy().into_owned();
        let control = target.join("control").to_string_lossy().into_owned();
        let assets = target.join("assets").to_string_lossy().into_owned();
        let values = [
            ("KNOWWEAVE_C4_PROBE_DESTINATION_ROOT", destination.as_str()),
            ("KNOWWEAVE_C4_PROBE_CONTROL_ROOT", control.as_str()),
            ("KNOWWEAVE_C4_PROBE_ASSET_ROOT", assets.as_str()),
            (
                "KNOWWEAVE_C4_PROBE_EXPECTED_DATABASE",
                "learning_restore_c4_2b8a1252-54d5-48aa-b176-a9586a86bea3",
            ),
        ];
        for missing in values.map(|(key, _)| key) {
            assert!(
                probe_config_from(|key| {
                    values
                        .iter()
                        .find(|(name, _)| *name == key && *name != missing)
                        .map(|(_, value)| (*value).to_owned())
                })
                .is_err()
            );
        }
        assert!(
            probe_config_from(|key| {
                values
                    .iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, value)| (*value).to_owned())
            })
            .is_ok()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires a new, running, root-owned isolated PG18 target and compile-time birth digest"]
    fn live_read_only_bound_target_probe() {
        let config = probe_config_from(|key| std::env::var(key).ok())
            .expect("explicit nonsecret bound-probe environment required");
        linux::probe_bound_target(&config).expect("bound target observation failed closed");
        println!("BOUND_TARGET_READ_ONLY_PG18_PASSED_NOT_RESTORE");
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires a NEW running root-owned isolated PG18 target and compile-time birth digest"]
    fn live_read_only_bound_target_guard() {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

        fn snapshot(
            config: &RestorePreflightConfig,
        ) -> std::collections::BTreeMap<String, Vec<u8>> {
            let mut files = std::collections::BTreeMap::new();
            for (prefix, path) in [
                ("control", &config.control_root),
                ("destination", &config.destination_root),
                ("assets", &config.asset_root),
            ] {
                for entry in std::fs::read_dir(path).unwrap() {
                    let entry = entry.unwrap();
                    let meta = std::fs::symlink_metadata(entry.path()).unwrap();
                    assert!(meta.is_file() && meta.len() <= 64 * 1024);
                    files.insert(
                        format!("{prefix}/{}", entry.file_name().to_str().unwrap()),
                        std::fs::read(entry.path()).unwrap(),
                    );
                }
            }
            files
        }
        fn open_lock(path: &Path) -> File {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
                .open(path)
                .unwrap()
        }
        fn try_lock(file: &File) -> io::Result<()> {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }

        let config = probe_config_from(|key| std::env::var(key).ok())
            .expect("explicit nonsecret bound-guard environment required");
        assert_eq!(unsafe { libc::geteuid() }, 0);
        let target = config.control_root.parent().unwrap();
        let global_path = target
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".restore-target.lock");
        let lock_name = format!("{}.restore.lock", config.expected_database);
        let lock_path = config.control_root.join(&lock_name);
        let attempt = config
            .control_root
            .join(restore_attempt_name(&config.expected_database).unwrap());
        let before = snapshot(&config);
        // Fresh target only: exactly the sealed birth, empty destination/assets.
        assert_eq!(
            before.keys().cloned().collect::<Vec<_>>(),
            [format!("control/{}.birth.json", config.expected_database)]
        );
        assert!(!lock_path.exists() && !attempt.exists());

        // A competing global descriptor must block before target lock creation.
        let blocker = open_lock(&global_path);
        try_lock(&blocker).unwrap();
        assert!(linux::acquire_for_restore(&config).is_err());
        assert_eq!(before, snapshot(&config));
        drop(blocker);

        let guard = linux::acquire_for_restore(&config).expect("bound guard acquisition failed");
        let owned = guard
            ._target_lock
            .as_ref()
            .expect("guard must own created target lock");
        let owned_meta = owned.metadata().unwrap();
        let path_meta = std::fs::symlink_metadata(&lock_path).unwrap();
        assert!(path_meta.is_file());
        assert_eq!(
            (owned_meta.dev(), owned_meta.ino()),
            (path_meta.dev(), path_meta.ino())
        );
        assert_eq!(path_meta.uid(), 0);
        assert_eq!(path_meta.nlink(), 1);
        assert_eq!(path_meta.permissions().mode() & 0o777, 0o600);
        assert_eq!(path_meta.len(), 0);
        let global_contender = open_lock(&global_path);
        let target_contender = open_lock(&lock_path);
        for file in [&global_contender, &target_contender] {
            assert_eq!(
                try_lock(file).unwrap_err().kind(),
                io::ErrorKind::WouldBlock
            );
        }
        guard
            .recheck()
            .expect("Docker/PG identity changed while held");
        linux::assert_guard_pg_empty(&guard.claim);
        assert!(guard_files_unchanged(
            &before,
            &snapshot(&config),
            &format!("control/{lock_name}")
        ));
        assert!(!attempt.exists());
        guard
            .recheck()
            .expect("Docker/PG identity changed after empty-data observation");
        drop(guard);
        for file in [&global_contender, &target_contender] {
            try_lock(file).expect("guard drop must release both flocks");
        }
        assert!(guard_files_unchanged(
            &before,
            &snapshot(&config),
            &format!("control/{lock_name}")
        ));
        assert!(!attempt.exists());
        println!("BOUND_TARGET_GUARD_READ_ONLY_PG18_PASSED_NOT_RESTORE");
    }

    const ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const NET: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
    const IMAGE: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";

    fn claim() -> DockerClaim {
        DockerClaim {
            container_id: ID.into(),
            network_id: NET.into(),
            image_id: IMAGE.into(),
            volume_name: "learning-system-p0c4-restore-2b8a1252-54d5-48aa-b176-a9586a86bea3_pg"
                .into(),
            mountpoint: "/var/lib/docker/volumes/x/_data".into(),
            project: "learning-system-p0c4-restore-2b8a1252-54d5-48aa-b176-a9586a86bea3".into(),
            network_name: "learning-system-p0c4-restore-2b8a1252-54d5-48aa-b176-a9586a86bea3_test"
                .into(),
            database: "learning_restore_c4_2b8a1252-54d5-48aa-b176-a9586a86bea3".into(),
            daemon_id: "DAEMON".into(),
            system_identifier: "7361082129910479001".into(),
            database_oid: 16385,
            subnet: "10.251.223.0/24".into(),
            target_path: "/var/lib/knowweave-c4/pin/targets/2b8a1252-54d5-48aa-b176-a9586a86bea3"
                .into(),
            initdb_source: "/var/lib/knowweave-c4/tools/initdb.sh".into(),
            mount_dev: 42,
            mount_ino: 43,
        }
    }

    #[test]
    fn fixed_exec_uses_exact_id_and_local_socket_without_shell_or_password() {
        let args = pg_exec_args(&claim()).unwrap();
        assert_eq!(&args[..6], ["exec", "-i", "--user", "postgres", ID, "psql"]);
        assert!(args.windows(2).any(|w| w == ["-h", "/var/run/postgresql"]));
        assert!(args.windows(2).any(|w| w == ["-d", &claim().database]));
        assert!(!args.iter().any(|a| a.contains("password") || a == "sh"));
        let mut bad = claim();
        bad.container_id = "pg".into();
        assert!(pg_exec_args(&bad).is_err());
    }

    #[test]
    fn exact_docker_identity_and_pg_facts_fail_closed_on_drift() {
        let c = claim();
        let good = json!({"Id":ID,"Image":IMAGE,"Config":{"Image":PINNED_IMAGE,
          "Labels":{"com.docker.compose.project":c.project,"com.docker.compose.service":"pg"}},
          "State":{"Running":true,"Health":{"Status":"healthy"},"StartedAt":"2026-09-28T00:00:00Z"},"RestartCount":0,
          "HostConfig":{"NetworkMode":c.network_name,"PortBindings":{}},
          "NetworkSettings":{"Ports":{},"Networks":{c.network_name.clone():{"NetworkID":NET}}},
          "Mounts":[{"Type":"volume","Name":c.volume_name,"Source":c.mountpoint,
                        "Destination":"/var/lib/postgresql","RW":true},
                    {"Type":"bind","Source":c.initdb_source,"Destination":"/docker-entrypoint-initdb.d/10-restore.sh","RW":false},
                    {"Type":"bind","Source":format!("{}/secrets/postgres_password",c.target_path),
                      "Destination":"/run/secrets/postgres_password","RW":false},
                    {"Type":"bind","Source":format!("{}/secrets/admin_password",c.target_path),
                      "Destination":"/run/secrets/admin_password","RW":false}]});
        let network = json!({"Id":NET,"Name":c.network_name,"Internal":true,
          "Labels":{"com.docker.compose.project":c.project},"IPAM":{"Config":[{"Subnet":"10.251.223.0/24"}]}});
        let mut network = network;
        network["Containers"] = json!({ID:{"Name":"pg"}});
        let volume = json!({"Name":c.volume_name,"Mountpoint":c.mountpoint,"Driver":"local","Scope":"local",
          "Options":{},"Labels":{"com.docker.compose.project":c.project}});
        assert!(validate_docker(&c, &good, &network, &volume).is_ok());
        let mut foreign_options = volume.clone();
        foreign_options["Options"] = json!({"device":"/other"});
        assert!(validate_docker(&c, &good, &network, &foreign_options).is_err());
        assert!(only_claimed_resource(&format!("{ID}\n"), ID).is_ok());
        assert!(only_claimed_resource(&format!("{ID}\n{NET}\n"), ID).is_err());
        let mut extra = good.clone();
        extra["Mounts"]
            .as_array_mut()
            .unwrap()
            .push(json!({"Type":"bind","Source":"/tmp/extra",
            "Destination":"/root/.pgpass","RW":false}));
        assert!(validate_docker(&c, &extra, &network, &volume).is_err());
        let mut foreign_network = network.clone();
        foreign_network["IPAM"]["Config"][0]["Subnet"] = json!("10.251.224.0/24");
        assert!(validate_docker(&c, &good, &foreign_network, &volume).is_err());
        let mut extra_member = network.clone();
        extra_member["Containers"][NET] = json!({"Name":"other"});
        assert!(validate_docker(&c, &good, &extra_member, &volume).is_err());
        for changed in [
            json!({"State":{"Running":false}}),
            json!({"Id":NET}),
            json!({"Image":"sha256:00"}),
        ] {
            let mut drift = good.clone();
            for (k, v) in changed.as_object().unwrap() {
                drift[k] = v.clone();
            }
            assert!(validate_docker(&c, &drift, &network, &volume).is_err());
        }
        assert!(validate_pg_line(&c, "16385|7361082129910479001\n").is_ok());
        assert!(validate_pg_line(&c, "16386|7361082129910479001\n").is_err());
        assert!(
            validate_pg_line(&c, "16385|7361082129910479001\n16385|7361082129910479001\n").is_err()
        );
        let before = validate_docker(&c, &good, &network, &volume).unwrap();
        let mut restarted = good.clone();
        restarted["RestartCount"] = json!(1);
        let after = validate_docker(&c, &restarted, &network, &volume).unwrap();
        assert!(same_observation(&before, &before).is_ok());
        assert!(same_observation(&before, &after).is_err());
    }

    #[test]
    fn absent_target_lock_does_not_create_a_file_or_mutate_control() {
        let root = std::env::temp_dir().join(format!(
            "c4-readonly-lock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let birth = root.join("birth.json");
        std::fs::write(&birth, b"sealed").unwrap();
        let before: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        let absent = root.join("target.restore.lock");
        let lock: Option<()> =
            optional_existing_lock(Err(io::Error::from(io::ErrorKind::NotFound)), || {
                panic!("missing lock must never be opened or created")
            })
            .unwrap();
        assert!(lock.is_none());
        assert!(!absent.exists());
        let after: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(before, after);
        assert_eq!(std::fs::read(&birth).unwrap(), b"sealed");
        assert!(optional_existing_lock::<(), _>(Ok(BackupEntryKind::Other), || Ok(())).is_err());
        assert!(optional_existing_lock(Ok(BackupEntryKind::File), || Ok(42)).unwrap() == Some(42));
        std::fs::remove_file(birth).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn precreation_requires_inode_provenance_and_only_pin_root_entries() {
        let c = claim();
        let root = Path::new(
            "/var/lib/knowweave-c4/pin-acceptance/batches/2b8a1252-54d5-48aa-b176-a9586a86bea3/control",
        );
        let data = json!({
            "format_version":1,"state":"PIN_ONLY_PRECREATION_NOT_RESTORE_AUTHORITY",
            "target_absent_at_precreation":true,
            "batch_id":"2b8a1252-54d5-48aa-b176-a9586a86bea3",
            "project":c.project,"database":c.database,"network":c.network_name,
            "volume":c.volume_name,"subnet":c.subnet,"root_path":root.to_string_lossy(),
            "root_dev":11,"root_ino":12,"targets_dev":13,"targets_ino":14,
            "docker_daemon_id":c.daemon_id,"initdb_path":c.initdb_source,
            "initdb_sha256":"a".repeat(64),
        });
        let canonical = serde_json::to_vec(&data).unwrap();
        assert!(parse_precreation(&canonical).is_ok());
        let mut with_space = canonical.clone();
        with_space.push(b' ');
        assert!(parse_precreation(&with_space).is_err());
        let mut extra_key = data.clone();
        extra_key["unreviewed"] = json!(true);
        assert!(parse_precreation(&serde_json::to_vec(&extra_key).unwrap()).is_err());
        let duplicated = String::from_utf8(canonical.clone()).unwrap().replacen(
            "\"format_version\":1",
            "\"format_version\":1,\"format_version\":1",
            1,
        );
        assert!(parse_precreation(duplicated.as_bytes()).is_err());
        let roots = vec![
            ".restore-target.lock".into(),
            "pin-precreation.json".into(),
            "targets".into(),
        ];
        let targets = vec!["2b8a1252-54d5-48aa-b176-a9586a86bea3".into()];
        let check = |value: Value, root: &Path, entries: &[String]| {
            let record: PinPrecreation = serde_json::from_value(value).unwrap();
            validate_precreation(&record, &c, root, (11, 12), (13, 14), entries, &targets)
        };
        assert!(check(data.clone(), root, &roots).is_ok());
        let mut wrong_inode = data.clone();
        wrong_inode["targets_ino"] = json!(15);
        assert!(check(wrong_inode, root, &roots).is_err());
        let mut wrong_version = data.clone();
        wrong_version["format_version"] = json!(2);
        assert!(check(wrong_version, root, &roots).is_err());
        let mut extra = roots.clone();
        extra.push("prior-result.json".into());
        assert!(check(data.clone(), root, &extra).is_err());
        let birth_root = Path::new("/var/lib/knowweave-c4/birth-acceptance/batches/x/control");
        let mut replay = data;
        replay["root_path"] = json!(birth_root.to_string_lossy());
        assert!(check(replay, birth_root, &roots).is_err());
    }
}

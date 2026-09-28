//! Root-only, read-only Docker/PG identity probe. This is intentionally not an
//! admission or restore API: the complete-backup and build-pin boundary remains
//! in `preflight_restore` until this probe can be joined to its lock lifetime.
use super::*;
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
        let file = dir.open_file(name)?;
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

    /// Internal read-only observation. It cannot authorize an import and is
    /// not wired into preflight until the full locked lifetime is designed.
    #[allow(dead_code)]
    pub(super) fn probe_bound_target(config: &RestorePreflightConfig) -> Result<(), BackupError> {
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
        let _global_lock = lock_existing(&root, ".restore-target.lock")?;
        let target = BackupDir::open_trusted_private_root(target_path)?;
        let control = BackupDir::open_trusted_private_root(&config.control_root)?;
        let lock_name = format!("{}.restore.lock", config.expected_database);
        // The standalone probe never creates the restore-attempt lock: a clean
        // pin candidate must retain a control root containing only birth JSON.
        // If another controller already created one, acquire it after the
        // global creation lock to preserve lock ordering.
        let _target_lock = optional_existing_lock(control.kind(&lock_name), || {
            lock_existing(&control, &lock_name)
        })?;
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
        let success_bytes = read_private_target_file(&target, "issuance-success.json", 4096)?;
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
        let precreation: PinPrecreation = serde_json::from_slice(&read_private_target_file(
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
        let targets_dir = root.open_dir("targets")?;
        validate_precreation(
            &precreation,
            &claim,
            root_path,
            root.identity()?,
            targets_dir.identity()?,
            &root.list()?,
            &targets_dir.list()?,
        )?;
        if reviewed_initdb(Path::new(&precreation.initdb_path))? != precreation.initdb_sha256 {
            return Err(BackupError::Invalid("bound initdb source differs"));
        }
        let before = observe(&claim)?;
        let sql = docker(&pg_exec_args(&claim)?)?;
        validate_pg_line(&claim, &sql)?;
        let after = observe(&claim)?;
        same_observation(&before, &after)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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

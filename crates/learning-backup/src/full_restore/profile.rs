//! Test-only fixed topology proof. Never a source capture or target authority.
use crate::BackupError;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
#[cfg(target_os = "linux")]
use std::{io::Read, path::Path};
#[cfg(target_os = "linux")]
const PATH: &str = "/var/lib/knowweave-full-rehearsal/profile.json";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    path: String,
    dev: u64,
    ino: u64,
    uid: u32,
    gid: u32,
    mode: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Volume {
    path: String,
    dev: u64,
    ino: u64,
    uid: u32,
    gid: u32,
    mode: u32,
    name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Tool {
    path: String,
    dev: u64,
    ino: u64,
    uid: u32,
    gid: u32,
    mode: u32,
    sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    format_version: u32,
    capability: String,
    source_case_id: String,
    target_case_id: String,
    source_container_id: String,
    target_container_id: String,
    source_network_id: String,
    target_network_id: String,
    daemon_id: String,
    application_build_sha256: String,
    target_birth_sha256: String,
    source_bind_root: Identity,
    initdb: Tool,
    volumes: BTreeMap<String, Volume>,
    docker_client: Tool,
    docker_socket: Identity,
}
pub(crate) struct FullRehearsalProfile {
    record: Record,
}
impl FullRehearsalProfile {
    #[cfg(target_os = "linux")]
    pub(crate) fn read_installed() -> Result<Self, BackupError> {
        let pin = option_env!("KNOWWEAVE_C4_FULL_PROFILE_SHA256").ok_or(BackupError::Invalid(
            "full rehearsal profile not build pinned",
        ))?;
        let dir = learning_assets::backup_fs::BackupDir::open_trusted_private_root(
            Path::new(PATH).parent().unwrap(),
        )?;
        let mut file = dir.open_file("profile.json")?;
        use std::os::unix::fs::MetadataExt;
        let meta = file.metadata()?;
        if meta.uid() != 0
            || meta.mode() & 0o7777 != 0o444
            || meta.nlink() != 1
            || meta.len() > 16384
        {
            return Err(BackupError::Invalid("private full rehearsal profile"));
        }
        let mut bytes = Vec::new();
        file.by_ref().take(16385).read_to_end(&mut bytes)?;
        let value = Self::parse(&bytes, pin)?;
        if option_env!("KNOWWEAVE_BUILD_ID_SHA256")
            != Some(value.record.application_build_sha256.as_str())
            || option_env!("KNOWWEAVE_C4_TARGET_BIRTH_SHA256")
                != Some(value.record.target_birth_sha256.as_str())
        {
            return Err(BackupError::Invalid(
                "full rehearsal build or birth differs",
            ));
        }
        value.recheck_files()?;
        Ok(value)
    }
    fn parse(bytes: &[u8], pin: &str) -> Result<Self, BackupError> {
        if bytes.is_empty()
            || bytes.len() > 16384
            || !crate::valid_digest(pin)
            || hex::encode(Sha256::digest(bytes)) != pin
        {
            return Err(BackupError::Invalid("full rehearsal profile pin"));
        }
        let raw: serde_json::Value = serde_json::from_slice(bytes)?;
        if serde_json::to_vec(&raw)? != bytes {
            return Err(BackupError::Invalid(
                "full rehearsal profile canonical bytes",
            ));
        }
        let record: Record = serde_json::from_value(raw)?;
        let parsed = Self { record };
        parsed.validate()?;
        Ok(parsed)
    }
    fn validate(&self) -> Result<(), BackupError> {
        let r = &self.record;
        if r.format_version != 1
            || r.capability != "full_rehearsal_profile_v1"
            || !crate::registry::v4(&r.source_case_id)
            || !crate::registry::v4(&r.target_case_id)
            || r.source_case_id == r.target_case_id
            || !crate::valid_digest(&r.application_build_sha256)
            || !crate::valid_digest(&r.target_birth_sha256)
            || r.daemon_id.is_empty()
            || r.daemon_id.len() > 128
            || !r
                .daemon_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
        {
            return Err(BackupError::Invalid("full rehearsal identity"));
        }
        let ids = [
            &r.source_container_id,
            &r.target_container_id,
            &r.source_network_id,
            &r.target_network_id,
        ];
        if ids.iter().any(|id| !crate::valid_digest(id))
            || ids.iter().collect::<std::collections::BTreeSet<_>>().len() != 4
        {
            return Err(BackupError::Invalid("full rehearsal resource identity"));
        }
        let source = format!("kwc4c-{}", r.source_case_id.replace('-', ""));
        let target = format!("learning-system-p0c4-restore-{}", r.target_case_id);
        let expected = [
            ("source_pg", format!("{source}-pg")),
            ("source_data", format!("{source}-source")),
            ("source_build", format!("{source}-build")),
            ("source_registry", format!("{source}-registry")),
            ("target_pg", format!("{target}_pg")),
            ("target_control", format!("{target}_control")),
            ("target_socket", format!("{target}_socket")),
        ];
        if r.volumes.len() != expected.len() {
            return Err(BackupError::Invalid("full rehearsal volume set"));
        }
        let mut identities = std::collections::BTreeSet::new();
        for (key, name) in expected {
            let v = r
                .volumes
                .get(key)
                .ok_or(BackupError::Invalid("full rehearsal volume absent"))?;
            if v.name != name
                || !crate::registry::canonical_path(&v.path)
                || !v.path.ends_with(&format!("/volumes/{name}/_data"))
                || v.dev == 0
                || v.ino == 0
                || v.mode > 0o7777
                || !identities.insert((v.dev, v.ino))
            {
                return Err(BackupError::Invalid("full rehearsal volume binding"));
            }
        }
        if r.volumes["target_socket"].uid != 999
            || r.volumes["target_socket"].mode != 0o3775
            || r.volumes["target_control"].uid != 0
            || r.volumes["target_control"].mode != 0o700
            || r.source_bind_root.uid != 0
            || r.source_bind_root.mode != 0o700
            || !r
                .source_bind_root
                .path
                .ends_with(&format!("/{}", r.source_case_id))
            || !crate::registry::canonical_path(&r.source_bind_root.path)
        {
            return Err(BackupError::Invalid("full rehearsal root shape"));
        }
        for tool in [&r.docker_client, &r.initdb] {
            if tool.uid != 0 || tool.dev == 0 || tool.ino == 0 || !crate::valid_digest(&tool.sha256)
            {
                return Err(BackupError::Invalid("full rehearsal tool identity"));
            }
        }
        if r.docker_client.path != "/usr/bin/docker"
            || !matches!(r.docker_client.mode, 0o555 | 0o755)
            || r.initdb.path != format!("{}/full-profile/initdb.sh", r.source_bind_root.path)
            || r.initdb.mode != 0o444
            || r.docker_socket.path != "/var/run/docker.sock"
            || r.docker_socket.uid != 0
            || !matches!(r.docker_socket.mode, 0o600 | 0o660)
        {
            return Err(BackupError::Invalid("full rehearsal tool path"));
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    fn recheck_files(&self) -> Result<(), BackupError> {
        use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
        let r = &self.record;
        for (key, path) in [
            ("source_pg", "/var/lib/postgresql"),
            ("source_data", "/var/lib/knowweave-source"),
            ("source_build", "/target"),
            ("source_registry", "/var/lib/knowweave-c4/registry"),
            ("target_socket", "/var/run/knowweave-target"),
            ("target_control", r.volumes["target_control"].path.as_str()),
            ("target_pg", r.volumes["target_pg"].path.as_str()),
        ] {
            let v = &r.volumes[key];
            let m = std::fs::symlink_metadata(path)?;
            if !m.is_dir()
                || (m.dev(), m.ino(), m.uid(), m.gid(), m.mode() & 0o7777)
                    != (v.dev, v.ino, v.uid, v.gid, v.mode)
            {
                return Err(BackupError::Invalid(
                    "full rehearsal mounted volume replaced",
                ));
            }
        }
        let s = &r.docker_socket;
        let m = std::fs::symlink_metadata(&s.path)?;
        if !m.file_type().is_socket()
            || (m.dev(), m.ino(), m.uid(), m.gid(), m.mode() & 0o7777)
                != (s.dev, s.ino, s.uid, s.gid, s.mode)
        {
            return Err(BackupError::Invalid("full rehearsal daemon socket differs"));
        }
        for (tool, path) in [
            (&r.docker_client, r.docker_client.path.as_str()),
            (&r.initdb, r.initdb.path.as_str()),
            (&r.initdb, "/var/lib/knowweave-full-rehearsal/initdb.sh"),
        ] {
            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(path)?;
            let m = file.metadata()?;
            if !m.is_file()
                || m.nlink() != 1
                || (m.dev(), m.ino(), m.uid(), m.gid(), m.mode() & 0o7777)
                    != (tool.dev, tool.ino, tool.uid, tool.gid, tool.mode)
                || m.len() > 256 * 1024 * 1024
            {
                return Err(BackupError::Invalid("full rehearsal tool replaced"));
            }
            let mut hash = Sha256::new();
            let mut buffer = [0u8; 65536];
            let mut size = 0u64;
            loop {
                let n = file.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                size += n as u64;
                if size > m.len() {
                    return Err(BackupError::Invalid("full rehearsal tool grew"));
                }
                hash.update(&buffer[..n]);
            }
            if size != m.len() || hex::encode(hash.finalize()) != tool.sha256 {
                return Err(BackupError::Invalid("full rehearsal tool bytes differ"));
            }
        }
        Ok(())
    }
    pub(crate) fn source_claim(
        &self,
        container: &str,
        project: &str,
        daemon: &str,
        network: &str,
        mounts: &str,
    ) -> Result<(), BackupError> {
        let r = &self.record;
        if container != r.source_container_id
            || project != format!("kwc4c-{}", r.source_case_id.replace('-', ""))
            || daemon != r.daemon_id
            || network != r.source_network_id
            || mounts != self.source_mount_hash()?
        {
            return Err(BackupError::Invalid("full rehearsal source claim differs"));
        }
        Ok(())
    }
    fn source_mount_hash(&self) -> Result<String, BackupError> {
        let r = &self.record;
        let root = &r.source_bind_root.path;
        let mut rows = Vec::new();
        for (key, dest, rw) in [
            ("source_pg", "/var/lib/postgresql", true),
            ("source_data", "/var/lib/knowweave-source", true),
            ("source_build", "/target", false),
            ("source_registry", "/var/lib/knowweave-c4/registry", true),
            (
                "target_control",
                r.volumes["target_control"].path.as_str(),
                true,
            ),
            ("target_socket", "/var/run/knowweave-target", false),
        ] {
            let v = &r.volumes[key];
            rows.push(serde_json::json!({"destination":dest,"source":v.path,"kind":"volume","rw":rw,"name":v.name}));
        }
        for (source, dest) in [
            (
                format!("{root}/initdb.sh"),
                "/docker-entrypoint-initdb.d/10-lifecycle.sh".to_owned(),
            ),
            ("/usr/bin/docker".into(), "/usr/bin/docker".into()),
            ("/var/run/docker.sock".into(), "/var/run/docker.sock".into()),
            (
                r.volumes["target_pg"].path.clone(),
                r.volumes["target_pg"].path.clone(),
            ),
            (
                format!("{root}/full-profile"),
                "/var/lib/knowweave-full-rehearsal".into(),
            ),
            (r.initdb.path.clone(), r.initdb.path.clone()),
            (
                format!("{root}/secrets/postgres_password"),
                "/run/secrets/postgres_password".into(),
            ),
            (
                format!("{root}/secrets/admin_password"),
                "/run/secrets/admin_password".into(),
            ),
        ] {
            rows.push(serde_json::json!({"destination":dest,"source":source,"kind":"bind","rw":false,"name":""}));
        }
        rows.sort_by(|a, b| a["destination"].as_str().cmp(&b["destination"].as_str()));
        Ok(hex::encode(Sha256::digest(serde_json::to_vec(&rows)?)))
    }
    pub(crate) fn target_container_id(&self) -> &str {
        &self.record.target_container_id
    }
    pub(crate) fn source_container_id(&self) -> &str {
        &self.record.source_container_id
    }
    pub(crate) fn target_socket(
        &self,
        container: &str,
        daemon: &str,
        network: &str,
        database: &str,
        volume: &str,
        root: &str,
    ) -> Result<serde_json::Value, BackupError> {
        let r = &self.record;
        if container != r.target_container_id
            || daemon != r.daemon_id
            || network != r.target_network_id
            || database != format!("learning_restore_c4_{}", r.target_case_id)
            || volume != r.volumes["target_pg"].name
            || root
                != format!(
                    "{}/targets/{}",
                    r.volumes["target_control"].path, r.target_case_id
                )
        {
            return Err(BackupError::Invalid("full rehearsal target differs"));
        }
        let v = &r.volumes["target_socket"];
        Ok(
            serde_json::json!({"Type":"volume","Source":v.path,"Destination":"/var/run/postgresql","RW":true,"Name":v.name}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record() -> serde_json::Value {
        serde_json::from_str(r#"{"application_build_sha256":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","capability":"full_rehearsal_profile_v1","daemon_id":"fixed-daemon","docker_client":{"dev":10,"gid":0,"ino":100,"mode":493,"path":"/usr/bin/docker","sha256":"1111111111111111111111111111111111111111111111111111111111111111","uid":0},"docker_socket":{"dev":10,"gid":0,"ino":100,"mode":432,"path":"/var/run/docker.sock","uid":0},"format_version":1,"initdb":{"dev":10,"gid":0,"ino":100,"mode":292,"path":"/private/full/550e8400-e29b-41d4-a716-446655440000/full-profile/initdb.sh","sha256":"2222222222222222222222222222222222222222222222222222222222222222","uid":0},"source_bind_root":{"dev":10,"gid":0,"ino":100,"mode":448,"path":"/private/full/550e8400-e29b-41d4-a716-446655440000","uid":0},"source_case_id":"550e8400-e29b-41d4-a716-446655440000","source_container_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","source_network_id":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","target_birth_sha256":"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","target_case_id":"650e8400-e29b-41d4-a716-446655440000","target_container_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","target_network_id":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","volumes":{"source_build":{"dev":10,"gid":0,"ino":102,"mode":448,"name":"kwc4c-550e8400e29b41d4a716446655440000-build","path":"/var/lib/docker/volumes/kwc4c-550e8400e29b41d4a716446655440000-build/_data","uid":0},"source_data":{"dev":10,"gid":0,"ino":101,"mode":448,"name":"kwc4c-550e8400e29b41d4a716446655440000-source","path":"/var/lib/docker/volumes/kwc4c-550e8400e29b41d4a716446655440000-source/_data","uid":0},"source_pg":{"dev":10,"gid":0,"ino":100,"mode":448,"name":"kwc4c-550e8400e29b41d4a716446655440000-pg","path":"/var/lib/docker/volumes/kwc4c-550e8400e29b41d4a716446655440000-pg/_data","uid":0},"source_registry":{"dev":10,"gid":0,"ino":103,"mode":448,"name":"kwc4c-550e8400e29b41d4a716446655440000-registry","path":"/var/lib/docker/volumes/kwc4c-550e8400e29b41d4a716446655440000-registry/_data","uid":0},"target_control":{"dev":10,"gid":0,"ino":105,"mode":448,"name":"learning-system-p0c4-restore-650e8400-e29b-41d4-a716-446655440000_control","path":"/var/lib/docker/volumes/learning-system-p0c4-restore-650e8400-e29b-41d4-a716-446655440000_control/_data","uid":0},"target_pg":{"dev":10,"gid":0,"ino":104,"mode":448,"name":"learning-system-p0c4-restore-650e8400-e29b-41d4-a716-446655440000_pg","path":"/var/lib/docker/volumes/learning-system-p0c4-restore-650e8400-e29b-41d4-a716-446655440000_pg/_data","uid":0},"target_socket":{"dev":10,"gid":999,"ino":106,"mode":2045,"name":"learning-system-p0c4-restore-650e8400-e29b-41d4-a716-446655440000_socket","path":"/var/lib/docker/volumes/learning-system-p0c4-restore-650e8400-e29b-41d4-a716-446655440000_socket/_data","uid":999}}}"#).unwrap()
    }
    fn parse(value: &serde_json::Value) -> Result<FullRehearsalProfile, BackupError> {
        let raw = serde_json::to_vec(value).unwrap();
        FullRehearsalProfile::parse(&raw, &hex::encode(Sha256::digest(&raw)))
    }
    #[test]
    fn exact_pin_and_canonical_bytes_are_mandatory() {
        let raw = serde_json::to_vec(&record()).unwrap();
        assert!(FullRehearsalProfile::parse(&raw, "").is_err());
        assert!(FullRehearsalProfile::parse(&raw, &"0".repeat(64)).is_err());
        let mut extra = raw.clone();
        extra.push(b'\n');
        assert!(FullRehearsalProfile::parse(&extra, &hex::encode(Sha256::digest(&extra))).is_err());
        assert!(parse(&record()).is_ok());
    }
    #[test]
    fn uuid_daemon_volume_tool_and_alias_changes_refuse() {
        let base = record();
        for (pointer, value) in [
            ("/target_case_id", base["source_case_id"].clone()),
            ("/daemon_id", serde_json::json!("")),
            ("/docker_client/path", serde_json::json!("/bin/sh")),
            (
                "/volumes/target_socket/name",
                serde_json::json!("old-socket"),
            ),
            ("/volumes/target_socket/uid", serde_json::json!(0)),
            (
                "/volumes/target_pg/ino",
                base["volumes"]["source_pg"]["ino"].clone(),
            ),
        ] {
            let mut value_record = base.clone();
            *value_record.pointer_mut(pointer).unwrap() = value;
            assert!(parse(&value_record).is_err(), "{pointer}");
        }
        let mut extra = base;
        extra["extra_mount"] = serde_json::json!("/host");
        assert!(parse(&extra).is_err());
    }
    #[test]
    fn source_projection_matches_python_and_target_network_is_bound() {
        let p = parse(&record()).unwrap();
        assert_eq!(
            p.source_mount_hash().unwrap(),
            "7f62ba9355a7afe67156722896c2c983be44882c1db672ac071660d3a23cd290"
        );
        let r = &p.record;
        let database = format!("learning_restore_c4_{}", r.target_case_id);
        let root = format!(
            "{}/targets/{}",
            r.volumes["target_control"].path, r.target_case_id
        );
        assert!(
            p.target_socket(
                &r.target_container_id,
                &r.daemon_id,
                &r.target_network_id,
                &database,
                &r.volumes["target_pg"].name,
                &root
            )
            .is_ok()
        );
        assert!(
            p.target_socket(
                &r.target_container_id,
                &r.daemon_id,
                &r.source_network_id,
                &database,
                &r.volumes["target_pg"].name,
                &root
            )
            .is_err()
        );
        assert!(
            p.source_claim(
                &r.source_container_id,
                &format!("kwc4c-{}", r.source_case_id.replace('-', "")),
                &r.daemon_id,
                &r.source_network_id,
                "7f62ba9355a7afe67156722896c2c983be44882c1db672ac071660d3a23cd290"
            )
            .is_ok()
        );
        assert!(
            p.source_claim(
                &r.source_container_id,
                &format!("kwc4c-{}", r.source_case_id.replace('-', "")),
                &r.daemon_id,
                &r.source_network_id,
                &"0".repeat(64)
            )
            .is_err()
        );
    }
}

//! Optional snapshot-effects v2 transport. S1 has no database or writer path.
//! A valid signal receives an explicit disabled/Unknown response, never a job
//! receipt, claim capability, NoEffect or AlreadyCommitted finding.
#[cfg(any(target_os = "linux", test))]
use serde::{Deserialize, Serialize};
use std::{io, path::PathBuf};
use uuid::Uuid;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
mod wire;
#[cfg(target_os = "linux")]
pub use linux::{DisabledBroker, signal_next};

pub const CLIENT_UID: u32 = 65532;
pub const CLIENT_GID: u32 = 65532;
pub const FRAME_CAP: usize = 4096;

#[must_use]
#[derive(Debug, PartialEq, Eq)]
pub enum SignalDisposition {
    WriterDisabledUnknown,
}

#[derive(Clone, Debug)]
pub struct ClientConfig {
    pub socket: PathBuf,
    pub epoch: Uuid,
    pub request_id: Uuid,
    pub lease_ms: u64,
}
impl ClientConfig {
    /// An explicit mode is parsed before the legacy worker opens any database
    /// or asset storage. Malformed explicit configuration never falls back.
    pub fn from_args(args: &[String]) -> Result<Option<Self>, String> {
        if args.first().map(String::as_str) != Some("--snapshot-broker-client") {
            return Ok(None);
        }
        let [
            _,
            socket_flag,
            socket,
            epoch_flag,
            epoch,
            request_flag,
            request,
            lease_flag,
            lease,
        ] = args
        else {
            return Err("invalid snapshot broker client arguments".into());
        };
        if socket_flag != "--socket"
            || epoch_flag != "--epoch"
            || request_flag != "--request-id"
            || lease_flag != "--lease-ms"
        {
            return Err("invalid snapshot broker client arguments".into());
        }
        let config = Self {
            socket: PathBuf::from(socket),
            epoch: canonical_id(epoch).map_err(|_| "invalid broker epoch")?,
            request_id: canonical_id(request).map_err(|_| "invalid broker request id")?,
            lease_ms: lease.parse().map_err(|_| "invalid broker lease duration")?,
        };
        config
            .validate()
            .map_err(|_| "invalid broker client configuration")?;
        Ok(Some(config))
    }

    pub(super) fn validate(&self) -> io::Result<()> {
        let expected_leaf = self.epoch.to_string();
        if self.epoch.is_nil()
            || self.request_id.is_nil()
            || !(30_000..=300_000).contains(&self.lease_ms)
            || !self.socket.is_absolute()
            || self
                .socket
                .file_name()
                .is_none_or(|name| name != "work.sock")
            || self
                .socket
                .parent()
                .and_then(|path| path.file_name())
                .is_none_or(|name| name != expected_leaf.as_str())
            || self.socket.components().any(|part| {
                !matches!(
                    part,
                    std::path::Component::RootDir | std::path::Component::Normal(_)
                )
            })
        {
            return Err(invalid());
        }
        Ok(())
    }
}

#[cfg(any(target_os = "linux", test))]
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Signal {
    format_version: u8,
    command: String,
    request_id: String,
    lease_ms: u64,
}
#[cfg(any(target_os = "linux", test))]
impl Signal {
    #[cfg(target_os = "linux")]
    fn for_client(config: &ClientConfig) -> Self {
        Self {
            format_version: 2,
            command: "SIGNAL_NEXT".into(),
            request_id: config.request_id.to_string(),
            lease_ms: config.lease_ms,
        }
    }

    fn parse(raw: &[u8]) -> io::Result<Self> {
        if raw.is_empty() || raw.len() > FRAME_CAP {
            return Err(invalid());
        }
        // serde's recursion limit remains enabled; this schema admits no
        // nested values. Duplicate known fields and all extra fields refuse.
        let value: Self = serde_json::from_slice(raw).map_err(|_| invalid())?;
        if value.format_version != 2
            || value.command != "SIGNAL_NEXT"
            || !(30_000..=300_000).contains(&value.lease_ms)
        {
            return Err(invalid());
        }
        canonical_id(&value.request_id)?;
        Ok(value)
    }
}

#[cfg(target_os = "linux")]
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DisabledReply {
    format_version: u8,
    epoch: String,
    request_id: String,
    disposition: String,
}
#[cfg(target_os = "linux")]
const DISABLED: &str = "WRITER_DISABLED_EFFECT_UNKNOWN_NO_JOB_EXECUTED";

fn canonical_id(value: &str) -> io::Result<Uuid> {
    let id = Uuid::parse_str(value).map_err(|_| invalid())?;
    if id.is_nil() || id.to_string() != value {
        return Err(invalid());
    }
    Ok(id)
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid snapshot broker protocol",
    )
}

#[cfg(not(target_os = "linux"))]
pub fn signal_next(_: &ClientConfig) -> io::Result<SignalDisposition> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "snapshot broker requires Linux",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUEST: &str = "00000000-0000-4000-8000-000000000001";

    #[test]
    fn strict_signal_rejects_duplicate_extra_identity_path_and_nested_values() {
        let good = format!(
            r#"{{"format_version":2,"command":"SIGNAL_NEXT","request_id":"{REQUEST}","lease_ms":30000}}"#
        );
        assert!(Signal::parse(good.as_bytes()).is_ok());
        for extra in [
            r#", "actor_id":"claimed-actor""#,
            r#", "job_id":"claimed-job""#,
            r#", "token":"claimed-token""#,
            r#", "path":"/source/export""#,
            r#", "lease_ms":30000"#,
        ] {
            let raw = format!("{}{extra}}}", &good[..good.len() - 1]);
            assert!(Signal::parse(raw.as_bytes()).is_err());
        }
        for bad in [
            good.replace("30000", "true"),
            good.replace("30000", "29999"),
            good.replace("30000", "300001"),
            good.replace("SIGNAL_NEXT", "CLAIM"),
            good.replace(REQUEST, "00000000-0000-0000-0000-000000000000"),
            good.replace("30000", &format!("{}0{}", "[".repeat(150), "]".repeat(150))),
            " ".repeat(FRAME_CAP + 1),
        ] {
            assert!(Signal::parse(bad.as_bytes()).is_err());
        }
    }

    #[test]
    fn explicit_invalid_client_configuration_cannot_become_legacy_mode() {
        assert!(
            ClientConfig::from_args(&["--run".into()])
                .unwrap()
                .is_none()
        );
        assert!(ClientConfig::from_args(&["--snapshot-broker-client".into()]).is_err());
        let args: Vec<String> = [
            "--snapshot-broker-client",
            "--socket",
            "/run/not-an-epoch/work.sock",
            "--epoch",
            REQUEST,
            "--request-id",
            REQUEST,
            "--lease-ms",
            "30000",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert!(ClientConfig::from_args(&args).is_err());
    }
}

//! Fixed privileged adapter. No caller-controlled command or environment API.
use super::*;
use sqlx::{Postgres, Row, Transaction};
use std::fs::File;

pub(super) async fn docker(args: &[String], deadline: Instant) -> Result<String, ChildFailure> {
    #[cfg(target_os = "linux")]
    super::super::linux::trusted_docker_path().map_err(|_| ChildFailure::Identity)?;
    #[cfg(not(target_os = "linux"))]
    check_linux_only()?;
    let mut command = tokio::process::Command::new("/usr/bin/docker");
    command
        .args(args)
        .env_clear()
        .env("DOCKER_HOST", "unix:///var/run/docker.sock");
    bounded_process::execute(&mut command, deadline, 1024 * 1024, 4096).await
}

#[cfg(not(target_os = "linux"))]
fn check_linux_only() -> Result<(), ChildFailure> {
    Err(ChildFailure::Identity)
}
fn mount_inode(claim: &DockerClaim) -> Result<(), ChildFailure> {
    #[cfg(target_os = "linux")]
    {
        super::super::linux::verify_mount_inode(claim).map_err(|_| ChildFailure::Identity)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = claim;
        check_linux_only()
    }
}

async fn inspect(kind: &str, id: &str, deadline: Instant) -> Result<Value, ChildFailure> {
    let output = docker(&[kind.into(), "inspect".into(), id.into()], deadline).await?;
    let value: Value = serde_json::from_str(&output).map_err(|_| ChildFailure::Protocol)?;
    let rows = value.as_array().ok_or(ChildFailure::Protocol)?;
    if rows.len() != 1 {
        return Err(ChildFailure::Protocol);
    }
    Ok(rows[0].clone())
}

async fn daemon(claim: &DockerClaim, deadline: Instant) -> Result<(), ChildFailure> {
    let output = docker(
        &["info".into(), "--format".into(), "{{.ID}}".into()],
        deadline,
    )
    .await?;
    if output.strip_suffix('\n') != Some(claim.daemon_id.as_str()) {
        return Err(ChildFailure::Identity);
    }
    Ok(())
}

async fn observe(claim: &DockerClaim, deadline: Instant) -> Result<Value, ChildFailure> {
    mount_inode(claim)?;
    daemon(claim, deadline).await?;
    let label = format!("label=com.docker.compose.project={}", claim.project);
    for (args, expected) in [
        (
            vec!["container", "ls", "-aq", "--no-trunc", "--filter", &label],
            &claim.container_id,
        ),
        (
            vec!["network", "ls", "-q", "--no-trunc", "--filter", &label],
            &claim.network_id,
        ),
        (
            vec!["volume", "ls", "-q", "--filter", &label],
            &claim.volume_name,
        ),
    ] {
        let args: Vec<_> = args.into_iter().map(str::to_owned).collect();
        only_claimed_resource(&docker(&args, deadline).await?, expected)
            .map_err(|_| ChildFailure::Identity)?;
    }
    let pg = inspect("container", &claim.container_id, deadline).await?;
    let network = inspect("network", &claim.network_id, deadline).await?;
    let volume = inspect("volume", &claim.volume_name, deadline).await?;
    let image = inspect("image", &claim.image_id, deadline).await?;
    let digest = PINNED_IMAGE.split_once('@').unwrap().1;
    if image.get("Id").and_then(Value::as_str) != Some(&claim.image_id)
        || !image
            .get("RepoDigests")
            .and_then(Value::as_array)
            .is_some_and(|rows| {
                rows.iter()
                    .any(|d| d.as_str().is_some_and(|s| s.ends_with(digest)))
            })
    {
        return Err(ChildFailure::Identity);
    }
    let projection =
        validate_docker(claim, &pg, &network, &volume).map_err(|_| ChildFailure::Identity)?;
    mount_inode(claim)?;
    Ok(projection)
}

trait QuarantineIo {
    async fn daemon(&mut self, claim: &DockerClaim, deadline: Instant) -> Result<(), ChildFailure>;
    async fn inspect(
        &mut self,
        claim: &DockerClaim,
        deadline: Instant,
    ) -> Result<Value, ChildFailure>;
    async fn stop(&mut self, claim: &DockerClaim, deadline: Instant) -> Result<(), ChildFailure>;
}
struct FixedQuarantine;
impl QuarantineIo for FixedQuarantine {
    async fn daemon(&mut self, claim: &DockerClaim, deadline: Instant) -> Result<(), ChildFailure> {
        daemon(claim, deadline).await
    }
    async fn inspect(
        &mut self,
        claim: &DockerClaim,
        deadline: Instant,
    ) -> Result<Value, ChildFailure> {
        inspect("container", &claim.container_id, deadline).await
    }
    async fn stop(&mut self, claim: &DockerClaim, deadline: Instant) -> Result<(), ChildFailure> {
        docker(
            &[
                "container".into(),
                "stop".into(),
                "--time".into(),
                "5".into(),
                claim.container_id.clone(),
            ],
            deadline,
        )
        .await
        .map(|_| ())
    }
}
async fn quarantine_with(claim: &DockerClaim, io: &mut impl QuarantineIo) -> Isolation {
    let deadline = Instant::now() + Duration::from_secs(15);
    let result = async {
        if !exact_id(&claim.container_id) {
            return Err(ChildFailure::Identity);
        }
        io.daemon(claim, deadline).await?;
        let before = io.inspect(claim, deadline).await?;
        if !eq_str(&before, &["Id"], &claim.container_id)
            || !eq_str(&before, &["Image"], &claim.image_id)
        {
            return Err(ChildFailure::Identity);
        }
        io.stop(claim, deadline).await?;
        let after = io.inspect(claim, deadline).await?;
        io.daemon(claim, deadline).await?;
        if !eq_str(&after, &["Id"], &claim.container_id)
            || !eq_str(&after, &["Image"], &claim.image_id)
            || val(&after, &["State", "Running"]) != Some(&Value::Bool(false))
            || val(&after, &["State", "Pid"]).and_then(Value::as_u64) != Some(0)
        {
            return Err(ChildFailure::Identity);
        }
        Ok(())
    }
    .await;
    if result.is_ok() {
        Isolation::Stopped
    } else {
        Isolation::UnconfirmedUnusable
    }
}
async fn quarantine(claim: &DockerClaim) -> Isolation {
    quarantine_with(claim, &mut FixedQuarantine).await
}

struct LinuxIo {
    cancellation_claim: Option<DockerClaim>,
}
impl Drop for LinuxIo {
    fn drop(&mut self) {
        if let Some(claim) = self.cancellation_claim.take() {
            // The guard was invalidated before the first await. Cancellation
            // cannot return proof; this detached bounded cleanup is best effort.
            // Runtime shutdown leaves it UNCONFIRMED_UNUSABLE, never reusable.
            tokio::spawn(async move {
                let _ = quarantine(&claim).await;
            });
        }
    }
}

impl ChildIo<Transaction<'static, Postgres>> for LinuxIo {
    async fn session(
        &mut self,
        lease: &mut Transaction<'static, Postgres>,
        deadline: Instant,
    ) -> Result<(i32, u64), ChildFailure> {
        let row = tokio::time::timeout_at(deadline.into(), sqlx::query(
            "SELECT pg_catalog.pg_backend_pid() AS backend_pid, d.oid::bigint AS database_oid FROM pg_catalog.pg_database d WHERE d.datname=pg_catalog.current_database()"
        ).fetch_one(&mut **lease)).await.map_err(|_|ChildFailure::Deadline)?.map_err(|_|ChildFailure::Session)?;
        let pid = row
            .try_get("backend_pid")
            .map_err(|_| ChildFailure::Session)?;
        let oid = u64::try_from(
            row.try_get::<i64, _>("database_oid")
                .map_err(|_| ChildFailure::Session)?,
        )
        .map_err(|_| ChildFailure::Session)?;
        Ok((pid, oid))
    }
    async fn observe(
        &mut self,
        claim: &DockerClaim,
        deadline: Instant,
    ) -> Result<Value, ChildFailure> {
        observe(claim, deadline).await
    }
    async fn run(&mut self, args: &[String], deadline: Instant) -> Result<String, ChildFailure> {
        docker(args, deadline).await
    }
    async fn quarantine(&mut self, claim: &DockerClaim) -> Isolation {
        quarantine(claim).await
    }
}

pub(in crate::restore_preflight) async fn attest<'a>(
    guard: &'a BoundTargetGuard<File, Option<File>>,
    challenge: &'a mut LockChallenge<Transaction<'static, Postgres>>,
) -> Result<
    ChildReadOnlyAttestation<'a, File, Option<File>, Transaction<'static, Postgres>>,
    ChildError,
> {
    // Validate before arming cancellation cleanup; an invalid ID never becomes
    // a stop target. A previously invalidated guard cannot run another probe.
    ExactRestoreChildTarget::bind(guard, challenge).map_err(|_| ChildError {
        reason: ChildFailure::Unusable,
        isolation: Isolation::UnconfirmedUnusable,
    })?;
    let mut io = LinuxIo {
        cancellation_claim: Some(guard.claim.clone()),
    };
    let result = attest_with(guard, challenge, &mut io).await;
    io.cancellation_claim = None;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    struct StopFixture {
        wrong_id: bool,
        wrong_daemon: bool,
        still_running: bool,
        residual_pid: bool,
        stop_fails: bool,
        inspections: usize,
        stops: Vec<String>,
    }
    impl StopFixture {
        fn good() -> Self {
            Self {
                wrong_id: false,
                wrong_daemon: false,
                still_running: false,
                residual_pid: false,
                stop_fails: false,
                inspections: 0,
                stops: vec![],
            }
        }
    }
    impl QuarantineIo for StopFixture {
        async fn daemon(&mut self, _: &DockerClaim, _: Instant) -> Result<(), ChildFailure> {
            if self.wrong_daemon {
                Err(ChildFailure::Identity)
            } else {
                Ok(())
            }
        }
        async fn inspect(
            &mut self,
            claim: &DockerClaim,
            _: Instant,
        ) -> Result<Value, ChildFailure> {
            self.inspections += 1;
            Ok(
                json!({"Id":if self.wrong_id {"other"}else{&claim.container_id},"Image":claim.image_id,"State":{"Running":self.inspections==1 || self.still_running,"Pid":if self.inspections==1 || self.residual_pid {123}else{0}}}),
            )
        }
        async fn stop(&mut self, claim: &DockerClaim, _: Instant) -> Result<(), ChildFailure> {
            self.stops.push(claim.container_id.clone());
            if self.stop_fails {
                Err(ChildFailure::Deadline)
            } else {
                Ok(())
            }
        }
    }
    #[tokio::test]
    async fn quarantine_requires_exact_identity_and_confirmed_zero_process_state() {
        let claim = super::super::super::tests::claim();
        let mut io = StopFixture::good();
        assert_eq!(quarantine_with(&claim, &mut io).await, Isolation::Stopped);
        assert_eq!(io.stops, std::slice::from_ref(&claim.container_id));
        for kind in 0..5 {
            let mut io = StopFixture::good();
            match kind {
                0 => io.wrong_id = true,
                1 => io.wrong_daemon = true,
                2 => io.still_running = true,
                3 => io.residual_pid = true,
                _ => io.stop_fails = true,
            };
            assert_eq!(
                quarantine_with(&claim, &mut io).await,
                Isolation::UnconfirmedUnusable
            );
            if kind < 2 {
                assert!(io.stops.is_empty());
            }
        }
    }
}

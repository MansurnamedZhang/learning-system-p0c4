use super::ChildFailure;
use std::process::Stdio;
use std::time::Instant;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

async fn limited(
    mut input: impl AsyncRead + Unpin,
    cap: usize,
    overflow: ChildFailure,
) -> Result<Vec<u8>, ChildFailure> {
    let mut result = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let n = input
            .read(&mut buffer)
            .await
            .map_err(|_| ChildFailure::Io)?;
        if n == 0 {
            return Ok(result);
        }
        if n > cap.saturating_sub(result.len()) {
            return Err(overflow);
        }
        result.extend_from_slice(&buffer[..n]);
    }
}

// Supervisor owns the process even if the awaiting caller is cancelled. On
// cancellation it kills and waits the host CLI instead of abandoning a handle.
pub(super) async fn execute(
    command: &mut Command,
    deadline: Instant,
    stdout_cap: usize,
    stderr_cap: usize,
) -> Result<String, ChildFailure> {
    if Instant::now() >= deadline {
        return Err(ChildFailure::Deadline);
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| ChildFailure::Io)?;
    let stdout = child.stdout.take().ok_or(ChildFailure::Io)?;
    let stderr = child.stderr.take().ok_or(ChildFailure::Io)?;
    let (mut sender, receiver) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let outcome = tokio::select! {
            _ = sender.closed() => Err(ChildFailure::Unusable),
            _ = tokio::time::sleep_until(deadline.into()) => Err(ChildFailure::Deadline),
            result = async {
                let (out,err,status) = tokio::try_join!(
                    limited(stdout,stdout_cap,ChildFailure::StdoutLimit),
                    limited(stderr,stderr_cap,ChildFailure::StderrLimit),
                    async {child.wait().await.map_err(|_|ChildFailure::Io)}
                )?;
                if !status.success() { return Err(ChildFailure::Exit); }
                if !err.is_empty() { return Err(ChildFailure::Stderr); }
                String::from_utf8(out).map_err(|_|ChildFailure::Protocol)
            } => result,
        };
        if outcome.is_err() {
            let _ = child.start_kill();
            // Always reap the exact spawned host process before reporting.
            if child.wait().await.is_err() {
                let _ = sender.send(Err(ChildFailure::Io));
                return;
            }
        }
        let _ = sender.send(outcome);
    });
    receiver.await.map_err(|_| ChildFailure::Io)?
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        time::{Duration, Instant},
    };
    use tokio::process::Command;

    #[test]
    fn process_fixture() {
        match std::env::var("C4_CHILD_UNIT_FIXTURE").as_deref() {
            Ok("stdout") => {
                let mut out = std::io::stdout().lock();
                loop {
                    if out.write_all(&[b'x'; 2048]).is_err() {
                        break;
                    }
                }
            }
            Ok("stderr") => {
                let mut err = std::io::stderr().lock();
                loop {
                    if err.write_all(&[b'x'; 2048]).is_err() {
                        break;
                    }
                }
            }
            Ok("sleep") => std::thread::sleep(Duration::from_secs(30)),
            Ok("secret") => {
                eprint!("password=do-not-leak");
                std::process::exit(2);
            }
            _ => {}
        }
    }
    async fn fixture(mode: &str, limit: usize) -> Result<String, ChildFailure> {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", "restore_preflight::target_binding::child_attestation::bounded_process::tests::process_fixture", "--nocapture"])
            .env_clear().env("C4_CHILD_UNIT_FIXTURE",mode);
        execute(
            &mut command,
            Instant::now() + Duration::from_millis(800),
            limit,
            limit,
        )
        .await
    }
    #[tokio::test]
    async fn process_limits_each_stream_while_child_is_still_running() {
        assert_eq!(
            fixture("stdout", 1024).await,
            Err(ChildFailure::StdoutLimit)
        );
        assert_eq!(
            fixture("stderr", 1024).await,
            Err(ChildFailure::StderrLimit)
        );
    }
    #[tokio::test]
    async fn process_deadline_reaps_host_child_and_redacts_exit_stderr() {
        let start = Instant::now();
        assert_eq!(fixture("sleep", 4096).await, Err(ChildFailure::Deadline));
        assert!(start.elapsed() < Duration::from_secs(5));
        let failure = fixture("secret", 4096).await;
        assert_eq!(failure, Err(ChildFailure::Exit));
        assert!(!format!("{failure:?}").contains("do-not-leak"));
    }
}

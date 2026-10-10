use super::ChildFailure;
use sha2::{Digest, Sha256};
use std::process::Stdio;
use std::time::Instant;
use tokio::io::AsyncWriteExt;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

/// Kept by the detached supervisor through cancellation, wait/reap and every
/// endpoint check. Callers cannot recover this authority from a failed run.
pub(crate) trait CaptureGuard: Send + 'static {
    fn recheck(&mut self) -> impl std::future::Future<Output = Result<(), ChildFailure>> + Send;
    #[cfg(test)]
    fn spawned(&mut self, _child: &mut tokio::process::Child) -> Result<(), ChildFailure> {
        Ok(())
    }
    #[cfg(test)]
    fn cleanup(&mut self, _reason: ChildFailure) -> impl std::future::Future<Output = ()> + Send {
        async {}
    }
    #[cfg(test)]
    fn settled(&mut self) -> impl std::future::Future<Output = Result<(), ChildFailure>> + Send {
        async { Ok(()) }
    }
}

#[derive(Debug)]
pub(crate) struct StreamObservation {
    pub bytes: u64,
    pub sha256: String,
    pub prefix: Vec<u8>,
    pub memory: Vec<u8>,
}

async fn stream_to_file(
    mut input: impl AsyncRead + Unpin,
    mut output: Option<&mut tokio::fs::File>,
    cap: u64,
) -> Result<StreamObservation, ChildFailure> {
    let mut prefix = Vec::new();
    let mut memory = Vec::new();
    let mut hash = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 65536];
    loop {
        let n = input
            .read(&mut buffer)
            .await
            .map_err(|_| ChildFailure::Io)?;
        if n == 0 {
            break;
        }
        if n as u64 > cap.saturating_sub(size) {
            return Err(ChildFailure::StdoutLimit);
        }
        prefix.extend_from_slice(&buffer[..n.min(5 - prefix.len())]);
        if let Some(output) = output.as_mut() {
            output
                .write_all(&buffer[..n])
                .await
                .map_err(|_| ChildFailure::Io)?;
        } else {
            memory.extend_from_slice(&buffer[..n]);
        }
        hash.update(&buffer[..n]);
        size += n as u64;
    }
    if let Some(output) = output.as_mut() {
        output.flush().await.map_err(|_| ChildFailure::Io)?;
        output.sync_all().await.map_err(|_| ChildFailure::Io)?;
    }
    Ok(StreamObservation {
        bytes: size,
        sha256: format!("{:x}", hash.finalize()),
        prefix,
        memory,
    })
}

pub(crate) async fn execute_stream<G: CaptureGuard>(
    command: &mut Command,
    output: std::fs::File,
    guard: G,
    deadline: Instant,
    stdout_cap: u64,
    stderr_cap: usize,
) -> Result<(G, StreamObservation), ChildFailure> {
    if Instant::now() >= deadline {
        return Err(ChildFailure::Deadline);
    }
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| ChildFailure::Io)?;
    supervise_stream(child, Some(output), guard, deadline, stdout_cap, stderr_cap)?
        .await
        .map_err(|_| ChildFailure::Io)?
}

pub(crate) async fn execute_guarded_text<G: CaptureGuard>(
    command: &mut Command,
    guard: G,
    deadline: Instant,
    stdout_cap: usize,
    stderr_cap: usize,
) -> Result<(G, String), ChildFailure> {
    if Instant::now() >= deadline {
        return Err(ChildFailure::Deadline);
    }
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| ChildFailure::Io)?;
    let (guard, observation) =
        supervise_stream(child, None, guard, deadline, stdout_cap as u64, stderr_cap)?
            .await
            .map_err(|_| ChildFailure::Io)??;
    let output = String::from_utf8(observation.memory).map_err(|_| ChildFailure::Protocol)?;
    Ok((guard, output))
}

type StreamReceiver<G> =
    tokio::sync::oneshot::Receiver<Result<(G, StreamObservation), ChildFailure>>;

fn supervise_stream<G: CaptureGuard>(
    mut child: tokio::process::Child,
    output: Option<std::fs::File>,
    mut guard: G,
    deadline: Instant,
    stdout_cap: u64,
    stderr_cap: usize,
) -> Result<StreamReceiver<G>, ChildFailure> {
    let stdout = child.stdout.take().ok_or(ChildFailure::Io)?;
    let stderr = child.stderr.take().ok_or(ChildFailure::Io)?;
    let (mut sender, receiver) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let mut output = output.map(tokio::fs::File::from_std);
        #[cfg(test)]
        let started = guard.spawned(&mut child);
        #[cfg(not(test))]
        let started: Result<(), ChildFailure> = Ok(());
        let mut outcome = {
            if let Err(reason) = started {
                Err(reason)
            } else {
                let capture = async {
                    let (out, err, status) = tokio::try_join!(
                        stream_to_file(stdout, output.as_mut(), stdout_cap),
                        limited(stderr, stderr_cap, ChildFailure::StderrLimit),
                        async { child.wait().await.map_err(|_| ChildFailure::Io) }
                    )?;
                    if !status.success() {
                        return Err(ChildFailure::Exit);
                    }
                    if !err.is_empty() {
                        return Err(ChildFailure::Stderr);
                    }
                    Ok(out)
                };
                tokio::pin!(capture);
                let mut interval = tokio::time::interval(std::time::Duration::from_millis(250));
                loop {
                    tokio::select! {
                        biased;
                        _ = sender.closed() => break Err(ChildFailure::Unusable),
                        _ = tokio::time::sleep_until(deadline.into()) => break Err(ChildFailure::Deadline),
                        result = &mut capture => break result,
                        _ = interval.tick() => {
                            let check_deadline = deadline.min(Instant::now() + std::time::Duration::from_secs(5));
                            let check = tokio::select! {
                                _ = sender.closed() => break Err(ChildFailure::Unusable),
                                check = tokio::time::timeout_at(check_deadline.into(), guard.recheck()) => check,
                            };
                            match check {
                                Ok(Ok(())) => (),
                                Ok(Err(reason)) => break Err(reason),
                                Err(_) => break Err(ChildFailure::Deadline),
                            }
                        }
                    }
                }
            }
        };
        if outcome.is_err() {
            #[cfg(test)]
            guard
                .cleanup(*outcome.as_ref().err().expect("checked error"))
                .await;
            let _ = child.start_kill();
            if child.wait().await.is_err() {
                outcome = Err(ChildFailure::Io);
            }
            // Cancellation can leave a Tokio file write in its blocking pool.
            // Keep the original admission/protection guard until that exact
            // descriptor's pending write has completed as well as child reap.
            if let Some(output) = output.as_mut()
                && output.flush().await.is_err()
            {
                outcome = Err(ChildFailure::Io);
            }
        }
        #[cfg(test)]
        if let Err(reason) = guard.settled().await {
            outcome = Err(reason);
        }
        // The authority is returned only after successful wait, otherwise it
        // is dropped here after reaping. A vanished receiver cannot regain it.
        let _ = sender.send(outcome.map(|observation| (guard, observation)));
    });
    Ok(receiver)
}

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
pub(crate) async fn execute(
    command: &mut Command,
    deadline: Instant,
    stdout_cap: usize,
    stderr_cap: usize,
) -> Result<String, ChildFailure> {
    if Instant::now() >= deadline {
        return Err(ChildFailure::Deadline);
    }
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| ChildFailure::Io)?;
    supervise(child, deadline, stdout_cap, stderr_cap)?
        .await
        .map_err(|_| ChildFailure::Io)?
}

// Private lifecycle helper also lets tests retain an independent OS process
// witness before transferring the spawned child to the real supervisor.
fn supervise(
    mut child: tokio::process::Child,
    deadline: Instant,
    stdout_cap: usize,
    stderr_cap: usize,
) -> Result<tokio::sync::oneshot::Receiver<Result<String, ChildFailure>>, ChildFailure> {
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
    Ok(receiver)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        time::{Duration, Instant},
    };
    use tokio::process::Command;

    #[tokio::test]
    async fn streaming_capture_limits_without_text_decoding() {
        struct Guard;
        impl CaptureGuard for Guard {
            async fn recheck(&mut self) -> Result<(), ChildFailure> {
                Ok(())
            }
        }
        let path = std::env::temp_dir().join(format!("c4-stream-{}", uuid::Uuid::new_v4()));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", "restore_preflight::target_binding::child_attestation::bounded_process::tests::process_fixture", "--nocapture"]).env_clear().env("C4_CHILD_UNIT_FIXTURE","stdout");
        let result = execute_stream(
            &mut command,
            file,
            Guard,
            Instant::now() + Duration::from_secs(5),
            1024,
            1024,
        )
        .await;
        assert!(matches!(result, Err(ChildFailure::StdoutLimit)));
        assert!(std::fs::metadata(&path).unwrap().len() <= 1024);
        std::fs::remove_file(path).unwrap();
    }

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
    async fn process_deadline_and_exit_redact_stderr() {
        let start = Instant::now();
        assert_eq!(fixture("sleep", 4096).await, Err(ChildFailure::Deadline));
        assert!(start.elapsed() < Duration::from_secs(5));
        let failure = fixture("secret", 4096).await;
        assert_eq!(failure, Err(ChildFailure::Exit));
        assert!(!format!("{failure:?}").contains("do-not-leak"));
    }
    // Hold a duplicate handle to the exact spawned Windows process, rather
    // than rediscovering a potentially reused PID. Linux kill(pid,0) also
    // observes zombies, so ESRCH after supervisor completion implies reaping.
    #[cfg(windows)]
    struct ProcessWitness {
        pid: u32,
        handle: std::os::windows::io::OwnedHandle,
    }
    #[cfg(windows)]
    impl ProcessWitness {
        fn new(child: &tokio::process::Child) -> Self {
            use std::os::windows::io::BorrowedHandle;
            let pid = child.id().unwrap();
            // SAFETY: child owns this live process handle for the borrow; the
            // cloned OwnedHandle remains independently valid until dropped.
            let handle = unsafe { BorrowedHandle::borrow_raw(child.raw_handle().unwrap()) }
                .try_clone_to_owned()
                .unwrap();
            Self { pid, handle }
        }
        fn exited(&self) -> bool {
            use std::os::windows::io::AsRawHandle;
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32;
            }
            // SAFETY: handle is the retained owned process handle. Timeout zero
            // is a nonblocking query; this never terminates arbitrary processes.
            let status = unsafe { WaitForSingleObject(self.handle.as_raw_handle(), 0) };
            assert!(
                status == 0 || status == 258,
                "PID {} wait failed: {status}",
                self.pid
            );
            status == 0
        }
    }
    #[cfg(any(windows, target_os = "linux"))]
    #[tokio::test]
    async fn streaming_cancellation_and_deadline_reap_before_authority_drop() {
        struct Authority {
            witness: ProcessWitness,
            dropped: Option<tokio::sync::oneshot::Sender<bool>>,
        }
        impl CaptureGuard for Authority {
            async fn recheck(&mut self) -> Result<(), ChildFailure> {
                Ok(())
            }
        }
        impl Drop for Authority {
            fn drop(&mut self) {
                let _ = self.dropped.take().unwrap().send(self.witness.exited());
            }
        }
        for cancel in [false, true] {
            let path =
                std::env::temp_dir().join(format!("c4-owned-stream-{}", uuid::Uuid::new_v4()));
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            let mut command = Command::new(std::env::current_exe().unwrap());
            command.args(["--exact", "restore_preflight::target_binding::child_attestation::bounded_process::tests::process_fixture", "--nocapture"])
                .env_clear().env("C4_CHILD_UNIT_FIXTURE","sleep").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
            let child = command.spawn().unwrap();
            let witness = ProcessWitness::new(&child);
            let (sender, dropped) = tokio::sync::oneshot::channel();
            let guard = Authority {
                witness,
                dropped: Some(sender),
            };
            let receiver = supervise_stream(
                child,
                Some(file),
                guard,
                Instant::now() + Duration::from_millis(800),
                4096,
                4096,
            )
            .unwrap();
            if cancel {
                drop(receiver);
            } else {
                assert!(matches!(
                    receiver.await.unwrap(),
                    Err(ChildFailure::Deadline)
                ));
            }
            assert!(
                tokio::time::timeout(Duration::from_secs(5), dropped)
                    .await
                    .unwrap()
                    .unwrap(),
                "admission authority dropped before child was reaped"
            );
            std::fs::remove_file(path).unwrap();
        }
    }
    #[cfg(target_os = "linux")]
    struct ProcessWitness {
        pid: u32,
    }
    #[cfg(target_os = "linux")]
    impl ProcessWitness {
        fn new(child: &tokio::process::Child) -> Self {
            Self {
                pid: child.id().unwrap(),
            }
        }
        fn exited(&self) -> bool {
            // SAFETY: signal 0 only queries existence of this spawned PID.
            if unsafe { libc::kill(self.pid as i32, 0) } == 0 {
                return false;
            }
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ESRCH)
            );
            true
        }
    }
    #[cfg(any(windows, target_os = "linux"))]
    fn observed_sleep(
        deadline: Instant,
    ) -> (
        ProcessWitness,
        tokio::sync::oneshot::Receiver<Result<String, ChildFailure>>,
    ) {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", "restore_preflight::target_binding::child_attestation::bounded_process::tests::process_fixture", "--nocapture"])
            .env_clear().env("C4_CHILD_UNIT_FIXTURE", "sleep")
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        let child = command.spawn().unwrap();
        let witness = ProcessWitness::new(&child);
        assert!(!witness.exited());
        (witness, supervise(child, deadline, 4096, 4096).unwrap())
    }
    #[cfg(any(windows, target_os = "linux"))]
    #[tokio::test]
    async fn real_supervisor_deadline_terminates_observed_host_pid() {
        let (witness, result) = observed_sleep(Instant::now() + Duration::from_millis(800));
        assert_eq!(result.await.unwrap(), Err(ChildFailure::Deadline));
        assert!(
            witness.exited(),
            "deadline returned while child PID was still alive"
        );
    }
    #[cfg(any(windows, target_os = "linux"))]
    #[tokio::test]
    async fn real_supervisor_cancellation_terminates_observed_host_pid() {
        let (witness, result) = observed_sleep(Instant::now() + Duration::from_secs(10));
        // Dropping the actual awaiting receiver is the cancellation boundary
        // used by execute; its supervisor has already taken ownership.
        drop(result);
        let cleanup_deadline = Instant::now() + Duration::from_secs(2);
        while !witness.exited() && Instant::now() < cleanup_deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(witness.exited(), "cancelled waiter left child PID alive");
    }
}

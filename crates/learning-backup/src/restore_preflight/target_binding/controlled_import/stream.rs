//! Private interactive process owner. Only this value can write stdin.
use super::ImportFailure;
use std::{
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdin, Command},
    sync::{mpsc, watch},
    task::JoinHandle,
};

pub(super) struct StreamBudget {
    deadline: Instant,
    stdout_cap: usize,
    stderr_cap: usize,
}

impl StreamBudget {
    pub(super) fn writer() -> Self {
        Self {
            deadline: Instant::now() + Duration::from_secs(45),
            stdout_cap: 8 * 1024,
            stderr_cap: 8 * 1024,
        }
    }
    pub(super) fn decoder() -> Self {
        Self {
            deadline: Instant::now() + Duration::from_secs(15),
            stdout_cap: 64 * 1024,
            stderr_cap: 8 * 1024,
        }
    }
    #[cfg(test)]
    fn test(duration: Duration, stdout_cap: usize, stderr_cap: usize) -> Self {
        Self {
            deadline: Instant::now() + duration,
            stdout_cap,
            stderr_cap,
        }
    }
}

enum Output {
    Line(Vec<u8>),
    Eof,
}

pub(super) struct StreamOwner {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: mpsc::UnboundedReceiver<Output>,
    failure: watch::Receiver<Option<ImportFailure>>,
    stdout_task: Option<JoinHandle<()>>,
    stderr_task: Option<JoinHandle<Result<bool, ImportFailure>>>,
    deadline: Instant,
}

fn fail(sender: &watch::Sender<Option<ImportFailure>>, failure: ImportFailure) {
    if sender.borrow().is_none() {
        sender.send_replace(Some(failure));
    }
}

async fn read_stdout(
    mut input: impl AsyncRead + Unpin,
    cap: usize,
    lines: mpsc::UnboundedSender<Output>,
    failure: watch::Sender<Option<ImportFailure>>,
) {
    let mut buffer = [0_u8; 4096];
    let mut current = Vec::new();
    let mut used = 0_usize;
    loop {
        match input.read(&mut buffer).await {
            Ok(0) => {
                if !current.is_empty() {
                    fail(&failure, ImportFailure::Protocol);
                }
                let _ = lines.send(Output::Eof);
                return;
            }
            Ok(n) => {
                if n > cap.saturating_sub(used) {
                    fail(&failure, ImportFailure::StdoutLimit);
                    return;
                }
                used += n;
                for &byte in &buffer[..n] {
                    current.push(byte);
                    if byte == b'\n' {
                        let _ = lines.send(Output::Line(std::mem::take(&mut current)));
                    }
                }
            }
            Err(_) => {
                fail(&failure, ImportFailure::Io);
                return;
            }
        }
    }
}

async fn read_stderr(
    mut input: impl AsyncRead + Unpin,
    cap: usize,
    failure: watch::Sender<Option<ImportFailure>>,
) -> Result<bool, ImportFailure> {
    let mut buffer = [0_u8; 4096];
    let mut used = 0_usize;
    loop {
        match input.read(&mut buffer).await {
            Ok(0) => return Ok(used != 0),
            Ok(n) => {
                if n > cap.saturating_sub(used) {
                    fail(&failure, ImportFailure::StderrLimit);
                    return Err(ImportFailure::StderrLimit);
                }
                used += n;
            }
            Err(_) => {
                fail(&failure, ImportFailure::Io);
                return Err(ImportFailure::Io);
            }
        }
    }
}

pub(super) fn spawn_stream(
    mut command: Command,
    budget: StreamBudget,
) -> Result<StreamOwner, ImportFailure> {
    if Instant::now() >= budget.deadline {
        return Err(ImportFailure::Deadline);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| ImportFailure::Io)?;
    let stdin = child.stdin.take().ok_or(ImportFailure::Io)?;
    let stdout = child.stdout.take().ok_or(ImportFailure::Io)?;
    let stderr = child.stderr.take().ok_or(ImportFailure::Io)?;
    let (lines_tx, lines_rx) = mpsc::unbounded_channel();
    let (failure_tx, failure_rx) = watch::channel(None);
    let stdout_task = tokio::spawn(read_stdout(
        stdout,
        budget.stdout_cap,
        lines_tx,
        failure_tx.clone(),
    ));
    let stderr_task = tokio::spawn(read_stderr(stderr, budget.stderr_cap, failure_tx));
    Ok(StreamOwner {
        child: Some(child),
        stdin: Some(stdin),
        stdout: lines_rx,
        failure: failure_rx,
        stdout_task: Some(stdout_task),
        stderr_task: Some(stderr_task),
        deadline: budget.deadline,
    })
}

impl StreamOwner {
    fn error(&self) -> Option<ImportFailure> {
        *self.failure.borrow()
    }

    pub(super) async fn send(&mut self, bytes: &[u8]) -> Result<(), ImportFailure> {
        if let Some(error) = self.error() {
            return Err(error);
        }
        let stdin = self.stdin.as_mut().ok_or(ImportFailure::Io)?;
        let result = tokio::select! {
            biased;
            _ = tokio::time::sleep_until(self.deadline.into()) => Err(ImportFailure::Deadline),
            changed = self.failure.changed() => {
                let _ = changed;
                Err(self.error().unwrap_or(ImportFailure::Io))
            }
            result = stdin.write_all(bytes) => result.map_err(|_| ImportFailure::Io),
        };
        result?;
        self.error().map_or(Ok(()), Err)
    }

    pub(super) async fn next_line(
        &mut self,
        deadline: Instant,
    ) -> Result<Option<Vec<u8>>, ImportFailure> {
        if let Some(error) = self.error() {
            return Err(error);
        }
        let effective = deadline.min(self.deadline);
        if Instant::now() >= effective {
            return Err(ImportFailure::Deadline);
        }
        let mut watching = true;
        loop {
            tokio::select! {
                biased;
                _ = tokio::time::sleep_until(effective.into()) => return Err(ImportFailure::Deadline),
                changed = self.failure.changed(), if watching => {
                    if let Some(error) = self.error() { return Err(error); }
                    if changed.is_err() { watching = false; }
                }
                output = self.stdout.recv() => {
                    if let Some(error) = self.error() { return Err(error); }
                    return match output {
                        Some(Output::Line(line)) => Ok(Some(line)),
                        Some(Output::Eof) => Ok(None),
                        None => Err(ImportFailure::Io),
                    };
                }
            }
        }
    }

    pub(super) async fn close_input(&mut self) -> Result<(), ImportFailure> {
        if Instant::now() >= self.deadline {
            return Err(ImportFailure::Deadline);
        }
        drop(self.stdin.take());
        self.error().map_or(Ok(()), Err)
    }

    pub(super) async fn kill_and_wait(&mut self) -> Result<(), ImportFailure> {
        drop(self.stdin.take());
        if let Some(child) = self.child.as_mut() {
            if child.start_kill().is_err() && !matches!(child.try_wait(), Ok(Some(_))) {
                // Keep the still-owned Child for Drop's emergency reaper.
                return Err(ImportFailure::Io);
            }
            child.wait().await.map_err(|_| ImportFailure::Io)?;
            self.child.take();
        }
        let _ = self.join_readers().await?;
        Ok(())
    }

    // Both handles are always awaited or aborted and awaited. A completed
    // child alone does not prove pipe EOF: a descendant may retain the writers.
    async fn join_readers(&mut self) -> Result<Result<bool, ImportFailure>, ImportFailure> {
        // Keep handles in the owner through every await. If an enclosing
        // decoder operation is cancelled, cleanup can resume settlement.
        let stdout_task = &mut self.stdout_task;
        let stderr_task = &mut self.stderr_task;
        let mut stdout_result = stdout_task.is_none().then_some(Ok(()));
        let mut stderr_result = stderr_task.is_none().then_some(Ok(Ok(false)));
        while stdout_task.is_some() || stderr_task.is_some() {
            if Instant::now() >= self.deadline {
                break;
            }
            let wait_stdout = stdout_task.is_some();
            let wait_stderr = stderr_task.is_some();
            tokio::select! {
                biased;
                _ = tokio::time::sleep_until(self.deadline.into()) => break,
                result = async { stdout_task.as_mut().unwrap().await }, if wait_stdout => {
                    stdout_task.take();
                    stdout_result = Some(result);
                }
                result = async { stderr_task.as_mut().unwrap().await }, if wait_stderr => {
                    stderr_task.take();
                    stderr_result = Some(result);
                }
            }
        }
        let mut aborted = false;
        if let Some(task) = stdout_task.as_ref()
            && !task.is_finished()
        {
            task.abort();
            aborted = true;
        }
        if let Some(task) = stderr_task.as_ref()
            && !task.is_finished()
        {
            task.abort();
            aborted = true;
        }
        if let Some(task) = stdout_task.as_mut() {
            stdout_result = Some(task.await);
            stdout_task.take();
        }
        if let Some(task) = stderr_task.as_mut() {
            stderr_result = Some(task.await);
            stderr_task.take();
        }
        if aborted {
            return Err(ImportFailure::Deadline);
        }
        stdout_result
            .ok_or(ImportFailure::Io)?
            .map_err(|_| ImportFailure::Io)?;
        stderr_result
            .ok_or(ImportFailure::Io)?
            .map_err(|_| ImportFailure::Io)
    }

    // Success requires the same child, both output readers and zero exit.
    pub(super) async fn finish(&mut self) -> Result<(), ImportFailure> {
        drop(self.stdin.take());
        if Instant::now() >= self.deadline {
            self.kill_and_wait().await?;
            return Err(ImportFailure::Deadline);
        }
        let child = self.child.as_mut().ok_or(ImportFailure::Io)?;
        let mut watching = true;
        let outcome = loop {
            break tokio::select! {
                biased;
                _ = tokio::time::sleep_until(self.deadline.into()) => Err(ImportFailure::Deadline),
                changed = self.failure.changed(), if watching => {
                    if let Some(error) = *self.failure.borrow() { Err(error) }
                    else { watching = changed.is_ok(); continue; }
                },
                result = child.wait() => result.map_err(|_| ImportFailure::Io),
            };
        };
        let status = match outcome {
            Ok(status) => status,
            Err(error) => {
                self.kill_and_wait().await?;
                return Err(error);
            }
        };
        self.child.take();
        let stderr_used = self.join_readers().await?;
        if Instant::now() >= self.deadline {
            return Err(ImportFailure::Deadline);
        }
        if let Some(error) = self.error() {
            return Err(error);
        }
        if !status.success() {
            return Err(ImportFailure::Exit);
        }
        if stderr_used? {
            return Err(ImportFailure::Stderr);
        }
        while let Ok(output) = self.stdout.try_recv() {
            if matches!(output, Output::Line(_)) {
                return Err(ImportFailure::Protocol);
            }
        }
        Ok(())
    }
}

impl Drop for StreamOwner {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            drop(self.stdin.take());
            let _ = child.start_kill();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = child.wait().await;
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn process_fixture() {
        let Ok(mode) = std::env::var("C4_STREAM_FIXTURE") else {
            return;
        };
        // Terminate libtest's unfinished `test ...` prefix before fixture bytes.
        std::io::stdout().write_all(b"\n").unwrap();
        match mode.as_str() {
            "ready" => {
                std::io::stdout().write_all(b"READY\n").unwrap();
                std::io::stdout().flush().unwrap();
                let mut input = Vec::new();
                std::io::stdin().read_to_end(&mut input).unwrap();
            }
            "stdout" | "stderr" => {
                let output: &mut dyn Write = if mode == "stdout" {
                    &mut std::io::stdout()
                } else {
                    &mut std::io::stderr()
                };
                loop {
                    if output.write_all(&[b'x'; 2048]).is_err() {
                        break;
                    }
                }
            }
            "blocked" => std::thread::sleep(Duration::from_secs(30)),
            "stdout_block" => {
                let _ = std::io::stdout().write_all(&[b'x'; 4096]);
                std::thread::sleep(Duration::from_secs(30));
            }
            "stderr_block" => {
                let _ = std::io::stderr().write_all(&[b'x'; 4096]);
                std::thread::sleep(Duration::from_secs(30));
            }
            "partial" => {
                std::io::stdout().write_all(b"partial").unwrap();
            }
            "crlf" => {
                std::io::stdout()
                    .write_all(b"KW_C4|bad|READY|1|2|3\r\n")
                    .unwrap();
            }
            "extra" => {
                std::io::stdout().write_all(b"READY\nEXTRA\n").unwrap();
                std::io::stdin().read_to_end(&mut Vec::new()).unwrap();
            }
            "stderr_nonempty" => {
                std::io::stderr().write_all(b"redacted secret").unwrap();
            }
            "read_three" => {
                let mut input = [0_u8; 3];
                std::io::stdin().read_exact(&mut input).unwrap();
                std::io::stdout().write_all(b"READ3\n").unwrap();
            }
            "nonzero" => {
                std::io::stdout().write_all(b"TERMINAL\n").unwrap();
                std::process::exit(7);
            }
            "pipe_holder" => {
                let release =
                    std::path::PathBuf::from(std::env::var_os("C4_STREAM_HOLDER_RELEASE").unwrap());
                std::fs::write(release.with_extension("started"), b"").unwrap();
                let limit = Instant::now() + Duration::from_secs(10);
                while !release.exists() && Instant::now() < limit {
                    std::thread::sleep(Duration::from_millis(10));
                }
                std::fs::write(release.with_extension("done"), b"").unwrap();
            }
            "spawn_pipe_holder" | "spawn_pipe_holder_block" => {
                let release = std::env::var_os("C4_STREAM_HOLDER_RELEASE").unwrap();
                let mut holder = std::process::Command::new(std::env::current_exe().unwrap());
                holder.args(["--exact", "restore_preflight::target_binding::controlled_import::stream::tests::process_fixture", "--nocapture"])
                    .env_clear().env("C4_STREAM_FIXTURE", "pipe_holder")
                    .env("C4_STREAM_HOLDER_RELEASE", release)
                    .stdin(Stdio::null()).stdout(Stdio::inherit()).stderr(Stdio::inherit());
                let mut holder_child = holder.spawn().unwrap();
                std::io::stdout().write_all(b"HOLDER_READY\n").unwrap();
                std::io::stdout().flush().unwrap();
                // The test kills this owner before release, leaving the
                // separately started holder as the surviving pipe writer.
                let _ = holder_child.wait();
            }
            _ => panic!("unknown fixture"),
        }
        std::process::exit(0);
    }

    fn fixture(
        mode: &str,
        deadline: Duration,
        stdout_cap: usize,
        stderr_cap: usize,
    ) -> StreamOwner {
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", "restore_preflight::target_binding::controlled_import::stream::tests::process_fixture", "--nocapture"])
            .env_clear().env("C4_STREAM_FIXTURE", mode);
        spawn_stream(
            command,
            StreamBudget::test(deadline, stdout_cap, stderr_cap),
        )
        .unwrap()
    }

    async fn fixture_line(owner: &mut StreamOwner) -> Result<Option<Vec<u8>>, ImportFailure> {
        loop {
            let line = owner
                .next_line(Instant::now() + Duration::from_secs(2))
                .await?;
            match &line {
                Some(value) if value == b"\n" || value == b"running 1 test\n"
                    || value.starts_with(b"test restore_preflight::target_binding::controlled_import::stream::tests::process_fixture ... ") => continue,
                _ => return Ok(line),
            }
        }
    }

    #[tokio::test]
    async fn ready_arrives_while_input_is_open() {
        let mut owner = fixture("ready", Duration::from_secs(3), 1024, 1024);
        assert_eq!(
            fixture_line(&mut owner).await,
            Ok(Some(b"READY\n".to_vec()))
        );
        owner.send(b"payload\n").await.unwrap();
        owner.close_input().await.unwrap();
        owner.finish().await.unwrap();
    }

    #[tokio::test]
    async fn stdout_and_stderr_limits_apply_while_running() {
        for (mode, expected) in [
            ("stdout", ImportFailure::StdoutLimit),
            ("stderr", ImportFailure::StderrLimit),
        ] {
            let mut owner = fixture(mode, Duration::from_secs(3), 1024, 1024);
            let observed = fixture_line(&mut owner).await;
            assert_eq!(observed, Err(expected));
            owner.kill_and_wait().await.unwrap();
        }
    }

    #[tokio::test]
    async fn blocked_stdin_obeys_total_deadline() {
        let mut owner = fixture("blocked", Duration::from_millis(250), 1024, 1024);
        assert_eq!(
            owner.send(&vec![b'x'; 8 * 1024 * 1024]).await,
            Err(ImportFailure::Deadline)
        );
        owner.kill_and_wait().await.unwrap();
    }

    #[tokio::test]
    async fn output_limit_interrupts_blocked_stdin_before_deadline() {
        for (mode, expected) in [
            ("stdout_block", ImportFailure::StdoutLimit),
            ("stderr_block", ImportFailure::StderrLimit),
        ] {
            let mut owner = fixture(mode, Duration::from_secs(3), 1024, 1024);
            let start = Instant::now();
            assert_eq!(
                owner.send(&vec![b'x'; 8 * 1024 * 1024]).await,
                Err(expected)
            );
            assert!(start.elapsed() < Duration::from_secs(2));
            owner.kill_and_wait().await.unwrap();
        }
    }

    #[tokio::test]
    async fn partial_line_extra_line_and_nonempty_stderr_cannot_finish_successfully() {
        let mut partial = fixture("partial", Duration::from_secs(3), 1024, 1024);
        assert_eq!(
            fixture_line(&mut partial).await,
            Err(ImportFailure::Protocol)
        );
        partial.kill_and_wait().await.unwrap();

        let mut extra = fixture("extra", Duration::from_secs(3), 1024, 1024);
        assert_eq!(
            fixture_line(&mut extra).await,
            Ok(Some(b"READY\n".to_vec()))
        );
        extra.close_input().await.unwrap();
        assert_eq!(extra.finish().await, Err(ImportFailure::Protocol));

        let mut stderr = fixture("stderr_nonempty", Duration::from_secs(3), 1024, 1024);
        assert_eq!(stderr.finish().await, Err(ImportFailure::Stderr));
    }

    #[tokio::test]
    async fn crlf_and_wrong_nonce_stay_raw_for_strict_receipt_parser() {
        use super::super::protocol::{Nonce, parse_writer_line};
        let mut crlf = fixture("crlf", Duration::from_secs(3), 1024, 1024);
        let line = fixture_line(&mut crlf).await.unwrap().unwrap();
        assert!(line.ends_with(b"\r\n"));
        assert_eq!(
            parse_writer_line(&line, &Nonce::random().unwrap()),
            Err(ImportFailure::Protocol)
        );
        crlf.finish().await.unwrap();
    }

    #[tokio::test]
    async fn a_terminal_line_and_eof_do_not_replace_zero_exit() {
        let mut owner = fixture("nonzero", Duration::from_secs(3), 1024, 1024);
        assert_eq!(
            fixture_line(&mut owner).await,
            Ok(Some(b"TERMINAL\n".to_vec()))
        );
        assert_eq!(
            owner
                .next_line(Instant::now() + Duration::from_secs(1))
                .await,
            Ok(None)
        );
        assert_eq!(owner.finish().await, Err(ImportFailure::Exit));
    }

    #[tokio::test]
    async fn partial_stdin_send_is_an_io_failure_after_attempt_boundary() {
        let mut owner = fixture("read_three", Duration::from_secs(3), 1024, 1024);
        assert_eq!(
            owner.send(&vec![b'C'; 8 * 1024 * 1024]).await,
            Err(ImportFailure::Io)
        );
        owner.kill_and_wait().await.unwrap();
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn kill_and_wait_reaps_the_exact_child_process() {
        use std::os::windows::io::{AsRawHandle, BorrowedHandle};
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32;
        }
        let mut owner = fixture("blocked", Duration::from_secs(3), 1024, 1024);
        let child = owner.child.as_ref().unwrap();
        // SAFETY: borrow is valid for the live child; the clone owns an independent process handle.
        let witness = unsafe { BorrowedHandle::borrow_raw(child.raw_handle().unwrap()) }
            .try_clone_to_owned()
            .unwrap();
        assert_eq!(
            unsafe { WaitForSingleObject(witness.as_raw_handle(), 0) },
            258
        );
        owner.kill_and_wait().await.unwrap();
        assert_eq!(
            unsafe { WaitForSingleObject(witness.as_raw_handle(), 0) },
            0
        );
    }

    #[tokio::test]
    async fn expired_finish_rejects_already_exited_and_drained_success() {
        for _ in 0..32 {
            let mut owner = fixture("ready", Duration::from_secs(3), 1024, 1024);
            assert_eq!(
                fixture_line(&mut owner).await,
                Ok(Some(b"READY\n".to_vec()))
            );
            owner.close_input().await.unwrap();
            owner.child.as_mut().unwrap().wait().await.unwrap();
            tokio::time::timeout(Duration::from_secs(2), async {
                while !owner.stdout_task.as_ref().unwrap().is_finished()
                    || !owner.stderr_task.as_ref().unwrap().is_finished()
                {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .unwrap();
            owner.deadline = Instant::now() - Duration::from_millis(1);
            assert_eq!(owner.finish().await, Err(ImportFailure::Deadline));
            assert!(owner.child.is_none());
            assert!(owner.stdout_task.is_none() && owner.stderr_task.is_none());
        }
    }

    async fn assert_bounded_pipe_holder_cleanup(
        mode: &str,
        finish: bool,
        abort_stdout: bool,
        abort_stderr: bool,
    ) {
        let release =
            std::env::temp_dir().join(format!("c4-task4-pipe-holder-{}", uuid::Uuid::new_v4()));
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", "restore_preflight::target_binding::controlled_import::stream::tests::process_fixture", "--nocapture"])
            .env_clear().env("C4_STREAM_FIXTURE", mode).env("C4_STREAM_HOLDER_RELEASE", &release);
        let mut owner = spawn_stream(
            command,
            StreamBudget::test(Duration::from_millis(300), 1024, 1024),
        )
        .unwrap();
        assert_eq!(
            fixture_line(&mut owner).await,
            Ok(Some(b"HOLDER_READY\n".to_vec()))
        );
        tokio::time::timeout(Duration::from_secs(1), async {
            while !release.with_extension("started").exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(!release.exists(), "holder was already released");
        assert!(owner.child.as_mut().unwrap().try_wait().unwrap().is_none());
        if !abort_stdout {
            assert!(!owner.stdout_task.as_ref().unwrap().is_finished());
        }
        if !abort_stderr {
            assert!(!owner.stderr_task.as_ref().unwrap().is_finished());
        }
        if abort_stdout {
            owner.stdout_task.as_ref().unwrap().abort();
        }
        if abort_stderr {
            owner.stderr_task.as_ref().unwrap().abort();
        }
        let result = tokio::time::timeout(Duration::from_secs(1), async {
            if finish {
                owner.finish().await
            } else {
                owner.kill_and_wait().await
            }
        })
        .await;
        std::fs::write(&release, b"").unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !release.with_extension("done").exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        std::fs::remove_file(&release).unwrap();
        std::fs::remove_file(release.with_extension("started")).unwrap();
        std::fs::remove_file(release.with_extension("done")).unwrap();
        if result.is_err() {
            // Make a failing RED test leave no detached reader task or owned host.
            if let Some(mut child) = owner.child.take() {
                let _ = child.start_kill();
                let _ = child.wait().await;
            }
            if let Some(task) = owner.stdout_task.take() {
                task.abort();
                let _ = task.await;
            }
            if let Some(task) = owner.stderr_task.take() {
                task.abort();
                let _ = task.await;
            }
            panic!("inherited output pipe blocked host cleanup past the bound");
        }
        assert_eq!(result.unwrap(), Err(ImportFailure::Deadline));
        assert!(owner.child.is_none(), "exact host was not reaped");
        assert!(
            owner.stdout_task.is_none() && owner.stderr_task.is_none(),
            "readers not settled"
        );
    }

    #[tokio::test]
    async fn inherited_pipe_writer_cannot_block_kill_cleanup() {
        assert_bounded_pipe_holder_cleanup("spawn_pipe_holder", false, false, false).await;
    }

    #[tokio::test]
    async fn inherited_pipe_writer_cannot_block_finish_error_cleanup() {
        assert_bounded_pipe_holder_cleanup("spawn_pipe_holder_block", true, false, false).await;
    }

    #[tokio::test]
    async fn failed_stdout_reader_does_not_detach_blocked_stderr_reader() {
        assert_bounded_pipe_holder_cleanup("spawn_pipe_holder", false, true, false).await;
    }

    #[tokio::test]
    async fn failed_stderr_reader_does_not_detach_blocked_stdout_reader() {
        assert_bounded_pipe_holder_cleanup("spawn_pipe_holder", false, false, true).await;
    }

    async fn assert_finish_settles_reaped_child_readers(
        cancel: bool,
        cleanup_only: bool,
        recover_cancelled_finish: bool,
        completed_stdout: Option<bool>,
    ) {
        use std::{future::Future, task::Poll};
        struct AuthorityAtDrop {
            stdout: tokio::task::AbortHandle,
            stderr: tokio::task::AbortHandle,
            settled: std::sync::Arc<std::sync::atomic::AtomicBool>,
        }
        impl Drop for AuthorityAtDrop {
            fn drop(&mut self) {
                self.settled.store(
                    self.stdout.is_finished() && self.stderr.is_finished(),
                    std::sync::atomic::Ordering::Release,
                );
            }
        }
        let release =
            std::env::temp_dir().join(format!("c4-task5-finish-{}", uuid::Uuid::new_v4()));
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", "restore_preflight::target_binding::controlled_import::stream::tests::process_fixture", "--nocapture"])
            .env_clear().env("C4_STREAM_FIXTURE","spawn_pipe_holder").env("C4_STREAM_HOLDER_RELEASE",&release);
        let mut owner = spawn_stream(
            command,
            StreamBudget::test(Duration::from_secs(2), 4096, 4096),
        )
        .unwrap();
        assert_eq!(
            fixture_line(&mut owner).await,
            Ok(Some(b"HOLDER_READY\n".to_vec()))
        );
        tokio::time::timeout(Duration::from_secs(1), async {
            while !release.with_extension("started").exists() {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
        owner.child.as_mut().unwrap().start_kill().unwrap();
        owner.child.as_mut().unwrap().wait().await.unwrap();
        // Exact host is reaped; only the separate real pipe holder remains.
        if let Some(stdout) = completed_stdout {
            // A completed reader JoinHandle (with cancellation error) must be
            // consumed once even while the other real pipe reader is blocked.
            if stdout {
                owner.stdout_task.as_ref().unwrap().abort();
            } else {
                owner.stderr_task.as_ref().unwrap().abort();
            }
            tokio::time::timeout(Duration::from_secs(1), async {
                while !if stdout {
                    owner.stdout_task.as_ref().unwrap().is_finished()
                } else {
                    owner.stderr_task.as_ref().unwrap().is_finished()
                } {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }
        assert_eq!(
            owner.stdout_task.as_ref().unwrap().is_finished(),
            completed_stdout == Some(true)
        );
        assert_eq!(
            owner.stderr_task.as_ref().unwrap().is_finished(),
            completed_stdout == Some(false)
        );
        let stdout = owner.stdout_task.as_ref().unwrap().abort_handle();
        let stderr = owner.stderr_task.as_ref().unwrap().abort_handle();
        let settled_at_authority_drop =
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let authority = AuthorityAtDrop {
            stdout: stdout.clone(),
            stderr: stderr.clone(),
            settled: settled_at_authority_drop.clone(),
        };
        owner.deadline = Instant::now() + Duration::from_millis(250);
        let (tx, mut rx) = watch::channel(false);
        let deadline = Instant::now()
            + if cancel {
                Duration::from_secs(1)
            } else {
                Duration::from_millis(30)
            };
        let result = if cleanup_only {
            owner.child.take();
            tokio::time::timeout(Duration::from_secs(1), owner.kill_and_wait())
                .await
                .unwrap()
        } else {
            let finish = async {
                if recover_cancelled_finish {
                    super::super::step(&mut rx, deadline, owner.finish()).await
                } else {
                    super::super::settle_finish(&mut rx, deadline, owner.finish()).await
                }
            };
            tokio::pin!(finish);
            // Poll into join_readers before triggering the external rejection.
            std::future::poll_fn(|cx| {
                assert!(finish.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            if cancel {
                tx.send_replace(true);
            } else {
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
            tokio::time::timeout(Duration::from_secs(1), finish)
                .await
                .unwrap()
        };
        if let Some(stdout) = completed_stdout {
            assert!(
                if stdout {
                    owner.stdout_task.is_none()
                } else {
                    owner.stderr_task.is_none()
                },
                "completed reader was not consumed before cancellation"
            );
        }
        let settled_before_cleanup = stdout.is_finished() && stderr.is_finished();
        let child_reaped = owner.child.is_none();
        // Mirror the Linux supervisor: cleanup precedes releasing authority.
        let _ = owner.kill_and_wait().await;
        drop(owner);
        drop(authority);
        // Explicit failing-RED hygiene: terminate detached readers if present,
        // then release only this test's holder and remove its own marker files.
        stdout.abort();
        stderr.abort();
        tokio::time::timeout(Duration::from_secs(1), async {
            while !stdout.is_finished() || !stderr.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        std::fs::write(&release, b"").unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !release.with_extension("done").exists() {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
        for path in [
            &release,
            &release.with_extension("started"),
            &release.with_extension("done"),
        ] {
            std::fs::remove_file(path).unwrap();
        }
        assert_eq!(
            result,
            Err(if cancel {
                ImportFailure::Cancelled
            } else {
                ImportFailure::Deadline
            })
        );
        assert!(child_reaped);
        assert!(
            (settled_before_cleanup || recover_cancelled_finish)
                && settled_at_authority_drop.load(std::sync::atomic::Ordering::Acquire),
            "reader tasks detached before authority release"
        );
    }

    #[tokio::test]
    async fn cancellation_during_finish_settles_reaped_child_readers() {
        assert_finish_settles_reaped_child_readers(true, false, false, None).await;
    }
    #[tokio::test]
    async fn external_deadline_during_finish_settles_reaped_child_readers() {
        assert_finish_settles_reaped_child_readers(false, false, false, None).await;
    }
    #[tokio::test]
    async fn cleanup_after_child_removed_settles_remaining_readers() {
        assert_finish_settles_reaped_child_readers(false, true, false, None).await;
    }
    #[tokio::test]
    async fn abandoned_finish_keeps_reader_handles_for_cleanup() {
        assert_finish_settles_reaped_child_readers(true, false, true, None).await;
    }
    #[tokio::test]
    async fn cancelled_finish_with_completed_stdout_recovers_stderr_once() {
        assert_finish_settles_reaped_child_readers(true, false, true, Some(true)).await;
    }
    #[tokio::test]
    async fn cancelled_finish_with_completed_stderr_recovers_stdout_once() {
        assert_finish_settles_reaped_child_readers(true, false, true, Some(false)).await;
    }
}

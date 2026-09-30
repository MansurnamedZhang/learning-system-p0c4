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
        let child = self.child.as_mut().ok_or(ImportFailure::Io)?;
        let _ = child.start_kill();
        child.wait().await.map_err(|_| ImportFailure::Io)?;
        self.child.take();
        if let Some(task) = self.stdout_task.take() {
            task.await.map_err(|_| ImportFailure::Io)?;
        }
        if let Some(task) = self.stderr_task.take() {
            let _ = task.await.map_err(|_| ImportFailure::Io)?;
        }
        Ok(())
    }

    // Success requires the same child, both output readers and zero exit.
    pub(super) async fn finish(&mut self) -> Result<(), ImportFailure> {
        drop(self.stdin.take());
        let child = self.child.as_mut().ok_or(ImportFailure::Io)?;
        let mut watching = true;
        let outcome = loop {
            break tokio::select! {
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
        let mut stdout_task = self.stdout_task.take().ok_or(ImportFailure::Io)?;
        let mut stderr_task = self.stderr_task.take().ok_or(ImportFailure::Io)?;
        let drained = tokio::select! {
            _ = tokio::time::sleep_until(self.deadline.into()) => None,
            result = async { tokio::try_join!(&mut stdout_task, &mut stderr_task) } => Some(result),
        };
        let (_, stderr_used) = match drained {
            Some(result) => result.map_err(|_| ImportFailure::Io)?,
            None => {
                stdout_task.abort();
                stderr_task.abort();
                let _ = tokio::join!(stdout_task, stderr_task);
                return Err(ImportFailure::Deadline);
            }
        };
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
}

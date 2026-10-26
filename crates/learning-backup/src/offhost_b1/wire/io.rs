//! Private pipe-only IO. No Child, SSH, identity, key or positive authority.
//! The conservative handshake clock stays active: untrusted syntax cannot
//! release it. This is NOT the original S admission or an authenticated epoch.
use super::{
    Direction, HEADER, Kind, SyntaxPhase, UntrustedFrame, WireError, WireSyntax, inspect_header,
    profile,
};
use std::{
    io,
    os::fd::{AsRawFd, OwnedFd},
    time::{Duration, Instant},
};
use tokio::io::unix::AsyncFd;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PipeError {
    Wire(WireError),
    UnsupportedDescriptor,
    SigpipePreconditionUnsatisfied,
    Readiness,
    Syscall,
    OperationDeadline,
    HandshakeDeadline,
    IdleDeadline,
    UnexpectedEof,
    TrailingData,
    WriteZero,
    BrokenPipe,
    Closed,
    Poisoned,
    Incomplete,
}
impl From<WireError> for PipeError {
    fn from(error: WireError) -> Self {
        Self::Wire(error)
    }
}

struct SessionClock {
    operation: Instant,
    handshake: Instant,
    idle: Instant,
}
impl SessionClock {
    fn start() -> Self {
        let now = Instant::now();
        Self {
            operation: now + Duration::from_secs(profile::OPERATION_SECONDS),
            handshake: now + Duration::from_secs(profile::HANDSHAKE_SECONDS),
            idle: now + Duration::from_secs(profile::IDLE_SECONDS),
        }
    }
    fn check(&self) -> Result<(), PipeError> {
        let now = Instant::now();
        if now >= self.operation {
            return Err(PipeError::OperationDeadline);
        }
        if now >= self.handshake {
            return Err(PipeError::HandshakeDeadline);
        }
        if now >= self.idle {
            return Err(PipeError::IdleDeadline);
        }
        Ok(())
    }
    fn deadline(&self) -> tokio::time::Instant {
        tokio::time::Instant::from_std(self.operation.min(self.handshake).min(self.idle))
    }
    fn progress(&mut self, bytes: usize) -> Result<(), PipeError> {
        self.check()?;
        if bytes > 0 {
            self.idle = Instant::now() + Duration::from_secs(profile::IDLE_SECONDS);
        }
        Ok(())
    }
    fn elapsed_error(&self) -> PipeError {
        self.check().err().unwrap_or(PipeError::Readiness)
    }
}

// These checks do not establish trusted process lineage or exclude external
// dup handles. No signal handler or descriptor flags are changed here.
fn sigpipe_ignored() -> Result<(), PipeError> {
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    if unsafe { libc::sigaction(libc::SIGPIPE, std::ptr::null(), &mut action) } != 0 {
        return Err(PipeError::Syscall);
    }
    if action.sa_sigaction != libc::SIG_IGN {
        return Err(PipeError::SigpipePreconditionUnsatisfied);
    }
    Ok(())
}
fn descriptor(fd: &OwnedFd, mode: i32) -> Result<(u64, u64), PipeError> {
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    let descriptor_flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) };
    if unsafe { libc::fstat(fd.as_raw_fd(), &mut stat) } != 0
        || flags < 0
        || descriptor_flags < 0
        || stat.st_mode & libc::S_IFMT != libc::S_IFIFO
        || flags & libc::O_ACCMODE != mode
        || flags & libc::O_NONBLOCK == 0
        || descriptor_flags & libc::FD_CLOEXEC == 0
    {
        return Err(PipeError::UnsupportedDescriptor);
    }
    Ok((stat.st_dev, stat.st_ino))
}
fn retry_without_progress(error: io::ErrorKind) -> bool {
    matches!(
        error,
        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
    )
}

async fn read_some(
    fd: &AsyncFd<OwnedFd>,
    clock: &mut SessionClock,
    buffer: &mut [u8],
) -> Result<usize, PipeError> {
    loop {
        clock.check()?;
        let mut ready = tokio::time::timeout_at(clock.deadline(), fd.readable())
            .await
            .map_err(|_| clock.elapsed_error())?
            .map_err(|_| PipeError::Readiness)?;
        clock.check()?;
        match ready.try_io(|fd| {
            let count = unsafe {
                libc::read(
                    fd.get_ref().as_raw_fd(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                )
            };
            if count < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(count as usize)
            }
        }) {
            // Return the actual count before the caller's post-syscall gate,
            // so a late positive read still leaves an honest retained offset.
            Ok(Ok(count)) => return Ok(count),
            Ok(Err(error)) if retry_without_progress(error.kind()) => continue,
            Ok(Err(_)) => return Err(PipeError::Syscall),
            Err(_) => continue,
        }
    }
}
async fn read_exact(
    fd: &AsyncFd<OwnedFd>,
    clock: &mut SessionClock,
    buffer: &mut [u8],
    progress: &mut usize,
) -> Result<(), PipeError> {
    let mut offset = 0;
    while offset < buffer.len() {
        let n = read_some(fd, clock, &mut buffer[offset..]).await?;
        offset += n;
        *progress += n;
        clock.progress(n)?;
        if n == 0 {
            return Err(PipeError::UnexpectedEof);
        }
    }
    clock.check()
}
async fn write_all(
    fd: &AsyncFd<OwnedFd>,
    clock: &mut SessionClock,
    buffer: &[u8],
    progress: &mut usize,
) -> Result<(), PipeError> {
    while *progress < buffer.len() {
        clock.check()?;
        sigpipe_ignored()?;
        let mut ready = tokio::time::timeout_at(clock.deadline(), fd.writable())
            .await
            .map_err(|_| clock.elapsed_error())?
            .map_err(|_| PipeError::Readiness)?;
        clock.check()?;
        sigpipe_ignored()?;
        match ready.try_io(|fd| {
            let tail = &buffer[*progress..];
            let count =
                unsafe { libc::write(fd.get_ref().as_raw_fd(), tail.as_ptr().cast(), tail.len()) };
            if count < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(count as usize)
            }
        }) {
            Ok(Ok(0)) => return Err(PipeError::WriteZero),
            Ok(Ok(count)) => {
                *progress += count;
                clock.progress(count)?;
            }
            Ok(Err(error)) if retry_without_progress(error.kind()) => continue,
            Ok(Err(error)) if error.raw_os_error() == Some(libc::EPIPE) => {
                return Err(PipeError::BrokenPipe);
            }
            Ok(Err(_)) => return Err(PipeError::Syscall),
            Err(_) => continue,
        }
    }
    clock.check()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UntrustedPipeTermination {
    EndFramesAndObservedEof,
    AbortAndObservedEof,
}
struct PipeIo {
    input: AsyncFd<OwnedFd>,
    output: Option<AsyncFd<OwnedFd>>,
    outbound: Direction,
    inbound: Direction,
    syntax: WireSyntax,
    clock: SessionClock,
    header: [u8; HEADER],
    buffer: Vec<u8>,
    read_progress: usize,
    write_progress: usize,
    inbound_end: bool,
    outbound_end: bool,
    eof_probe_started: bool,
    eof_observed: bool,
    probe: [u8; 1],
}
impl PipeIo {
    fn new(input: OwnedFd, output: OwnedFd, outbound: Direction) -> Result<Self, PipeError> {
        let reader = descriptor(&input, libc::O_RDONLY)?;
        let writer = descriptor(&output, libc::O_WRONLY)?;
        if reader == writer {
            return Err(PipeError::UnsupportedDescriptor);
        }
        sigpipe_ignored()?;
        let clock = SessionClock::start();
        Ok(Self {
            input: AsyncFd::new(input).map_err(|_| PipeError::Readiness)?,
            output: Some(AsyncFd::new(output).map_err(|_| PipeError::Readiness)?),
            outbound,
            inbound: match outbound {
                Direction::SourceToDestination => Direction::DestinationToSource,
                Direction::DestinationToSource => Direction::SourceToDestination,
            },
            syntax: WireSyntax::default(),
            clock,
            header: [0; HEADER],
            buffer: Vec::new(),
            read_progress: 0,
            write_progress: 0,
            inbound_end: false,
            outbound_end: false,
            eof_probe_started: false,
            eof_observed: false,
            probe: [0],
        })
    }
    fn check(&mut self) -> Result<(), PipeError> {
        if self.syntax.phase == SyntaxPhase::Poisoned {
            return Err(PipeError::Poisoned);
        }
        if let Err(error) = self.clock.check() {
            self.syntax.poison();
            return Err(error);
        }
        Ok(())
    }
    fn reject<T>(&mut self, error: PipeError) -> Result<T, PipeError> {
        self.syntax.poison();
        Err(error)
    }
    async fn read_frame(&mut self) -> Result<UntrustedFrame<'_>, PipeError> {
        self.check()?;
        if self.inbound_end || self.eof_probe_started {
            return self.reject(PipeError::Closed);
        }
        let mut reservation = self.syntax.reserve_frame(self.inbound)?;
        self.header.fill(0);
        self.read_progress = 0;
        read_exact(
            &self.input,
            &mut self.clock,
            &mut self.header,
            &mut self.read_progress,
        )
        .await?;
        let header = reservation.declare(&self.header)?;
        let length = HEADER
            .checked_add(
                usize::try_from(header.payload_bytes)
                    .map_err(|_| PipeError::Wire(WireError::Length))?,
            )
            .ok_or(PipeError::Wire(WireError::Length))?;
        self.buffer.resize(length, 0);
        self.buffer[..HEADER].copy_from_slice(&self.header);
        read_exact(
            &self.input,
            &mut self.clock,
            &mut self.buffer[HEADER..],
            &mut self.read_progress,
        )
        .await?;
        let frame = reservation.stage(&self.buffer)?;
        self.clock.check()?;
        reservation.finish()?;
        self.inbound_end = header.kind == Kind::End;
        Ok(frame)
    }
    async fn write_frame(&mut self, raw: &[u8]) -> Result<(), PipeError> {
        self.check()?;
        if self.outbound_end || self.output.is_none() {
            return self.reject(PipeError::Closed);
        }
        let header = match raw
            .get(..HEADER)
            .ok_or(WireError::Header)
            .and_then(inspect_header)
        {
            Ok(value) => value,
            Err(error) => return self.reject(PipeError::Wire(error)),
        };
        if header.direction != self.outbound {
            return self.reject(PipeError::Wire(WireError::Fields));
        }
        if header.kind == Kind::End
            && self.outbound == Direction::DestinationToSource
            && !self.eof_observed
        {
            return self.reject(PipeError::Incomplete);
        }
        let mut reservation = self.syntax.reserve_frame(self.outbound)?;
        reservation.declare(&raw[..HEADER])?;
        if usize::try_from(header.payload_bytes)
            .ok()
            .and_then(|n| n.checked_add(HEADER))
            != Some(raw.len())
        {
            return Err(PipeError::Wire(WireError::Length));
        }
        self.buffer.clear();
        self.buffer.extend_from_slice(raw);
        reservation.stage(&self.buffer)?;
        self.write_progress = 0;
        let output = self.output.as_ref().ok_or(PipeError::Closed)?;
        write_all(
            output,
            &mut self.clock,
            &self.buffer,
            &mut self.write_progress,
        )
        .await?;
        self.clock.check()?;
        reservation.finish()?;
        self.outbound_end = header.kind == Kind::End;
        Ok(())
    }
    async fn expect_eof(&mut self) -> Result<(), PipeError> {
        self.check()?;
        if self.eof_probe_started
            || (!self.inbound_end && self.syntax.phase != SyntaxPhase::Aborted)
        {
            return self.reject(PipeError::Incomplete);
        }
        let reservation = self.syntax.reserve_eof()?;
        self.eof_probe_started = true;
        let count = read_some(&self.input, &mut self.clock, &mut self.probe).await?;
        self.clock.progress(count)?;
        if count != 0 {
            return Err(PipeError::TrailingData);
        }
        self.clock.check()?;
        reservation.finish()?;
        self.eof_observed = true;
        Ok(())
    }
    fn close_output_after_end(&mut self) -> Result<(), PipeError> {
        self.check()?;
        if self.output.is_none()
            || (!self.outbound_end && self.syntax.phase != SyntaxPhase::Aborted)
        {
            return self.reject(PipeError::Incomplete);
        }
        // Closes this owned pipe write end only; no claim about external dup FDs.
        drop(self.output.take());
        self.clock.check().inspect_err(|_| self.syntax.poison())
    }
    fn finish_untrusted(&mut self) -> Result<UntrustedPipeTermination, PipeError> {
        self.check()?;
        if !self.eof_observed || self.output.is_some() {
            return self.reject(PipeError::Incomplete);
        }
        match self.syntax.phase {
            SyntaxPhase::BothEndsSeen => Ok(UntrustedPipeTermination::EndFramesAndObservedEof),
            SyntaxPhase::Aborted => Ok(UntrustedPipeTermination::AbortAndObservedEof),
            _ => self.reject(PipeError::Incomplete),
        }
    }
}

#[cfg(test)]
mod tests;

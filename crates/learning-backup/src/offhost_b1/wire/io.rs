//! Private pipe-only IO. No Child, SSH, identity, key or positive authority.
//! The conservative handshake clock stays active: untrusted syntax cannot
//! release it. This is NOT the original S admission or an authenticated epoch.
use super::{
    Direction, HEADER, Kind, SyntaxPhase, UntrustedFrame, WireError, WireSyntax, inspect_header,
    profile,
};
use std::{
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::io::unix::AsyncFd;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PipeError {
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

#[derive(Clone, Copy)]
enum OriginalClock {
    Ordinary,
    Source(&'static crate::offhost_b1::source_actor::SourceFrontOriginal),
    Destination(&'static crate::offhost_b1::destination_actor::OriginalDEntryScope),
}
struct SessionClock {
    operation: Instant,
    handshake: Instant,
    idle: Instant,
    original: OriginalClock,
    authenticated_handshake: bool,
}
impl SessionClock {
    fn start() -> Self {
        let now = Instant::now();
        Self {
            operation: now + Duration::from_secs(profile::OPERATION_SECONDS),
            handshake: now + Duration::from_secs(profile::HANDSHAKE_SECONDS),
            idle: now + Duration::from_secs(profile::IDLE_SECONDS),
            original: OriginalClock::Ordinary,
            authenticated_handshake: false,
        }
    }
    fn check(&self) -> Result<(), PipeError> {
        match self.original {
            OriginalClock::Ordinary => {}
            OriginalClock::Source(original) => {
                original
                    .revalidate()
                    .map_err(|_| PipeError::OperationDeadline)?;
                original
                    .first_s_clock()
                    .and_then(|clock| clock.check())
                    .map_err(|_| PipeError::OperationDeadline)?;
            }
            OriginalClock::Destination(original) => original
                .recheck()
                .map_err(|_| PipeError::OperationDeadline)?,
        }
        let now = Instant::now();
        if now >= self.operation {
            return Err(PipeError::OperationDeadline);
        }
        if !self.authenticated_handshake && now >= self.handshake {
            return Err(PipeError::HandshakeDeadline);
        }
        if now >= self.idle {
            return Err(PipeError::IdleDeadline);
        }
        Ok(())
    }
    fn deadline(&self) -> tokio::time::Instant {
        let operation = match self.original {
            OriginalClock::Source(original) => original
                .first_s_clock()
                .and_then(|clock| clock.deadline())
                .unwrap_or(self.operation)
                .min(self.operation),
            _ => self.operation,
        };
        let limit = if self.authenticated_handshake {
            operation.min(self.idle)
        } else {
            operation.min(self.handshake).min(self.idle)
        };
        tokio::time::Instant::from_std(limit)
    }
    fn prepay(&self, bytes: u64, entries: usize) -> Result<(), PipeError> {
        match self.original {
            OriginalClock::Ordinary => Ok(()),
            OriginalClock::Source(original) => original
                .prepay_wire_work(bytes, entries)
                .map_err(|_| PipeError::Wire(WireError::Budget)),
            OriginalClock::Destination(original) => original
                .prepay_wire_work(bytes, entries)
                .map_err(|_| PipeError::Wire(WireError::Budget)),
        }
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
fn descriptor(fd: &impl AsRawFd, mode: i32) -> Result<(u64, u64), PipeError> {
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

async fn read_some<F: AsRawFd>(
    fd: &AsyncFd<F>,
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
async fn read_exact<F: AsRawFd>(
    fd: &AsyncFd<F>,
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
async fn write_all<F: AsRawFd>(
    fd: &AsyncFd<F>,
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
pub(super) enum UntrustedPipeTermination {
    EndFramesAndObservedEof,
    AbortAndObservedEof,
}
pub(in crate::offhost_b1) struct PipeIo<F: AsRawFd = OwnedFd> {
    input: AsyncFd<F>,
    output: Option<AsyncFd<F>>,
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
    original_pair: Option<&'static OriginalPipePair>,
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
            original_pair: None,
        })
    }
}
impl<F: AsRawFd> PipeIo<F> {
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
    pub(super) async fn read_frame(&mut self) -> Result<UntrustedFrame<'_>, PipeError> {
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
        self.clock.prepay(
            (length as u64)
                .checked_mul(64)
                .ok_or(PipeError::Wire(WireError::Budget))?,
            16,
        )?;
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
    pub(super) async fn write_frame(&mut self, raw: &[u8]) -> Result<(), PipeError> {
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
        self.clock.prepay(
            (raw.len() as u64)
                .checked_mul(64)
                .ok_or(PipeError::Wire(WireError::Budget))?,
            16,
        )?;
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
    pub(super) async fn expect_eof(&mut self) -> Result<(), PipeError> {
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
    pub(super) fn close_output_after_end(&mut self) -> Result<(), PipeError> {
        self.check()?;
        if self.output.is_none()
            || (!self.outbound_end && self.syntax.phase != SyntaxPhase::Aborted)
        {
            return self.reject(PipeError::Incomplete);
        }
        // Closes this owned pipe write end only; no claim about external dup FDs.
        drop(self.output.take());
        if let Some(original) = self.original_pair {
            original.close_output_after_original_end()?;
        }
        self.clock.check().inspect_err(|_| self.syntax.poison())
    }
    pub(super) fn finish_untrusted(&mut self) -> Result<UntrustedPipeTermination, PipeError> {
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

/// Kernel descriptors are owned by the original process book, not by a
/// cancellable readiness registration. No FD constructor escapes this module.
struct OriginalPipeSlot {
    fd: Mutex<Option<OwnedFd>>,
}
impl AsRawFd for OriginalPipeSlot {
    fn as_raw_fd(&self) -> i32 {
        self.fd
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(AsRawFd::as_raw_fd))
            .unwrap_or(-1)
    }
}
#[derive(Clone, Copy)]
enum OriginalPipeOrigin {
    Source(&'static crate::offhost_b1::source_actor::SourceFrontOriginal),
    Destination(&'static crate::offhost_b1::destination_actor::OriginalDEntryScope),
}
pub(in crate::offhost_b1) struct OriginalPipePair {
    input: OriginalPipeSlot,
    output: OriginalPipeSlot,
    origin: OriginalPipeOrigin,
    registered: AtomicBool,
}
static D_STDIO_ENTERED: AtomicBool = AtomicBool::new(false);
static D_STDIO: OnceLock<OriginalPipePair> = OnceLock::new();
static S_STDIO_ENTERED: AtomicBool = AtomicBool::new(false);
static S_STDIO: OnceLock<OriginalPipePair> = OnceLock::new();
fn original_pipe_error() -> crate::BackupError {
    crate::BackupError::Invalid("B1 original pipe custody or lifecycle")
}
impl OriginalPipePair {
    fn prepay(&self, bytes: u64, entries: usize) -> Result<(), crate::BackupError> {
        match self.origin {
            OriginalPipeOrigin::Source(original) => original.prepay_wire_work(bytes, entries),
            OriginalPipeOrigin::Destination(original) => original.prepay_wire_work(bytes, entries),
        }
    }
    fn recheck(&self) -> Result<(), crate::BackupError> {
        self.prepay(0, 8)?;
        let read = descriptor(&self.input, libc::O_RDONLY).map_err(|_| original_pipe_error())?;
        let write = descriptor(&self.output, libc::O_WRONLY).map_err(|_| original_pipe_error())?;
        if read == write {
            return Err(original_pipe_error());
        }
        sigpipe_ignored().map_err(|_| original_pipe_error())
    }
    fn set_fixed_flags(&self) -> Result<(), crate::BackupError> {
        for slot in [&self.input, &self.output] {
            self.prepay(0, 4)?;
            let fd = slot.as_raw_fd();
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            let fdflags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
            if flags < 0
                || fdflags < 0
                || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
                || unsafe { libc::fcntl(fd, libc::F_SETFD, fdflags | libc::FD_CLOEXEC) } < 0
            {
                return Err(original_pipe_error());
            }
        }
        self.recheck()
    }
    fn close_output_after_original_end(&self) -> Result<(), PipeError> {
        let mut slot = self.output.fd.lock().map_err(|_| PipeError::Closed)?;
        let actual = slot.take().ok_or(PipeError::Closed)?;
        drop(actual);
        Ok(())
    }
}
pub(in crate::offhost_b1) fn retain_original_destination_stdio(
    original: &'static crate::offhost_b1::destination_actor::OriginalDEntryScope,
) -> Result<&'static OriginalPipePair, crate::BackupError> {
    original.recheck()?;
    if D_STDIO_ENTERED.swap(true, Ordering::AcqRel) {
        original.reject();
        return Err(original_pipe_error());
    }
    original.prepay_wire_work(std::mem::size_of::<OriginalPipePair>() as u64, 4)?;
    // Actual stdin/stdout are taken exactly once and retained BEFORE flags,
    // fstat or any other fallible local validation. No caller FD is accepted.
    let held = D_STDIO.get_or_init(|| OriginalPipePair {
        input: OriginalPipeSlot {
            fd: Mutex::new(Some(unsafe { OwnedFd::from_raw_fd(0) })),
        },
        output: OriginalPipeSlot {
            fd: Mutex::new(Some(unsafe { OwnedFd::from_raw_fd(1) })),
        },
        origin: OriginalPipeOrigin::Destination(original),
        registered: AtomicBool::new(false),
    });
    if let Err(error) = held.set_fixed_flags().and_then(|()| original.recheck()) {
        original.reject();
        return Err(error);
    }
    Ok(held)
}
pub(in crate::offhost_b1) fn retain_original_source_pipes(
    original: &'static crate::offhost_b1::source_actor::SourceFrontOriginal,
    ssh: &'static crate::offhost_b1::supervisor::OriginalSshOwner,
) -> Result<&'static OriginalPipePair, crate::BackupError> {
    original.revalidate()?;
    ssh.require_original(original)?;
    if S_STDIO_ENTERED.swap(true, Ordering::AcqRel) {
        original.reject();
        return Err(original_pipe_error());
    }
    original.prepay_wire_work(std::mem::size_of::<OriginalPipePair>() as u64, 4)?;
    let held = S_STDIO.get_or_init(|| OriginalPipePair {
        input: OriginalPipeSlot {
            fd: Mutex::new(None),
        },
        output: OriginalPipeSlot {
            fd: Mutex::new(None),
        },
        origin: OriginalPipeOrigin::Source(original),
        registered: AtomicBool::new(false),
    });
    {
        let mut slot = held.input.fd.lock().map_err(|_| original_pipe_error())?;
        if slot.is_some() {
            return Err(original_pipe_error());
        }
        *slot = Some(ssh.take_original_read_pipe()?);
    }
    {
        let mut slot = held.output.fd.lock().map_err(|_| original_pipe_error())?;
        if slot.is_some() {
            return Err(original_pipe_error());
        }
        *slot = Some(ssh.take_original_write_pipe()?);
    }
    if let Err(error) = held.set_fixed_flags().and_then(|()| original.revalidate()) {
        original.reject();
        return Err(error);
    }
    Ok(held)
}
pub(in crate::offhost_b1) fn close_original_source_output_negative(
    ssh: &crate::offhost_b1::supervisor::OriginalSshOwner,
    original: &'static crate::offhost_b1::source_actor::SourceFrontOriginal,
) -> Result<(), crate::BackupError> {
    original.require_negative_m6_custody(ssh)?;
    if let Some(pair) = S_STDIO.get() {
        if !matches!(pair.origin,OriginalPipeOrigin::Source(held) if std::ptr::eq(held,original)) {
            return Err(original_pipe_error());
        }
        drop(
            pair.output
                .fd
                .lock()
                .map_err(|_| original_pipe_error())?
                .take(),
        );
    }
    Ok(())
}
pub(in crate::offhost_b1) struct OriginalPipeBorrow {
    slot: &'static OriginalPipeSlot,
}
impl AsRawFd for OriginalPipeBorrow {
    fn as_raw_fd(&self) -> i32 {
        self.slot.as_raw_fd()
    }
}
pub(in crate::offhost_b1) type OriginalPipeIo = PipeIo<OriginalPipeBorrow>;
impl PipeIo<OriginalPipeBorrow> {
    fn register_original(
        pair: &'static OriginalPipePair,
        outbound: Direction,
        clock: SessionClock,
    ) -> Result<Self, crate::BackupError> {
        pair.recheck()?;
        if pair.registered.swap(true, Ordering::AcqRel) {
            return Err(original_pipe_error());
        }
        pair.prepay(std::mem::size_of::<Self>() as u64, 4)?;
        Ok(Self {
            input: AsyncFd::new(OriginalPipeBorrow { slot: &pair.input })
                .map_err(|_| original_pipe_error())?,
            output: Some(
                AsyncFd::new(OriginalPipeBorrow { slot: &pair.output })
                    .map_err(|_| original_pipe_error())?,
            ),
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
            original_pair: Some(pair),
        })
    }
    pub(in crate::offhost_b1) fn for_original_source(
        pair: &'static OriginalPipePair,
        first: &crate::offhost_b1::source_actor::SourceFirstS<'_>,
    ) -> Result<Self, crate::BackupError> {
        let original = first.original_owner();
        if !matches!(pair.origin, OriginalPipeOrigin::Source(held) if std::ptr::eq(held, original))
        {
            return Err(original_pipe_error());
        }
        let epoch = first.clock()?;
        let clock = SessionClock {
            operation: epoch.deadline()?,
            handshake: epoch.handshake_deadline_data()?,
            idle: Instant::now() + Duration::from_secs(profile::IDLE_SECONDS),
            original: OriginalClock::Source(original),
            authenticated_handshake: false,
        };
        Self::register_original(pair, Direction::SourceToDestination, clock)
    }
    pub(super) fn for_original_destination(
        pair: &'static OriginalPipePair,
        original: &mut crate::offhost_b1::destination_actor::OriginalDestinationSession<'_>,
    ) -> Result<Self, crate::BackupError> {
        let entry = {
            let mut view = original.wire_identity()?;
            view.recheck()?;
            view.entry_original()
        };
        if !matches!(pair.origin, OriginalPipeOrigin::Destination(held) if std::ptr::eq(held, entry))
        {
            return Err(original_pipe_error());
        }
        let operation = entry.operation_deadline_data();
        let handshake = operation
            .checked_sub(Duration::from_secs(profile::OPERATION_SECONDS))
            .and_then(|start| start.checked_add(Duration::from_secs(profile::HANDSHAKE_SECONDS)))
            .ok_or_else(original_pipe_error)?;
        let clock = SessionClock {
            operation,
            handshake,
            idle: Instant::now() + Duration::from_secs(profile::IDLE_SECONDS),
            original: OriginalClock::Destination(entry),
            authenticated_handshake: false,
        };
        Self::register_original(pair, Direction::DestinationToSource, clock)
    }
    /// Same original IO is retained by its process book. Crypto is checked
    /// against PREVIOUS transcript heads before syntax or D has adopted data.
    pub(super) async fn read_original_destination_frame(
        &mut self,
        original: &mut crate::offhost_b1::destination_actor::OriginalDestinationSession<'_>,
    ) -> Result<(UntrustedFrame<'_>, Option<super::VerifiedOriginalDControl>), crate::BackupError>
    {
        self.check().map_err(|_| original_pipe_error())?;
        if self.inbound_end || self.eof_probe_started {
            return Err(original_pipe_error());
        }
        let mut reservation = self
            .syntax
            .reserve_frame(self.inbound)
            .map_err(|_| original_pipe_error())?;
        self.header.fill(0);
        self.read_progress = 0;
        read_exact(
            &self.input,
            &mut self.clock,
            &mut self.header,
            &mut self.read_progress,
        )
        .await
        .map_err(|_| original_pipe_error())?;
        let header = reservation
            .declare(&self.header)
            .map_err(|_| original_pipe_error())?;
        let length = HEADER
            .checked_add(header.payload_bytes as usize)
            .ok_or_else(original_pipe_error)?;
        self.clock
            .prepay(
                (length as u64)
                    .checked_mul(64)
                    .ok_or_else(original_pipe_error)?,
                24,
            )
            .map_err(|_| original_pipe_error())?;
        self.buffer.resize(length, 0);
        self.buffer[..HEADER].copy_from_slice(&self.header);
        read_exact(
            &self.input,
            &mut self.clock,
            &mut self.buffer[HEADER..],
            &mut self.read_progress,
        )
        .await
        .map_err(|_| original_pipe_error())?;
        let parsed = super::parse_frame(&self.buffer, header).map_err(|_| original_pipe_error())?;
        let signed = matches!(
            &parsed.message,
            super::UntrustedMessage::Data(
                super::UntrustedControl::SourceAuth(_)
                    | super::UntrustedControl::SourceFinalDecision(_),
                _
            )
        );
        let verified = if signed {
            Some(super::verify_original_destination_control(
                reservation.owner,
                parsed,
                &self.buffer[..HEADER],
                &self.buffer[HEADER..],
                original,
            )?)
        } else {
            None
        };
        let frame = reservation
            .stage(&self.buffer)
            .map_err(|_| original_pipe_error())?;
        self.clock.check().map_err(|_| original_pipe_error())?;
        reservation.finish().map_err(|_| original_pipe_error())?;
        self.inbound_end = header.kind == Kind::End;
        Ok((frame, verified))
    }
    pub(super) fn mark_original_handshake_authenticated(
        &mut self,
    ) -> Result<(), crate::BackupError> {
        self.clock.check().map_err(|_| original_pipe_error())?;
        if matches!(self.clock.original, OriginalClock::Ordinary)
            || self.clock.authenticated_handshake
        {
            return Err(original_pipe_error());
        }
        self.clock.authenticated_handshake = true;
        Ok(())
    }
    pub(in crate::offhost_b1) fn close_output_negative_after_original_worker(
        &mut self,
        ssh: &crate::offhost_b1::supervisor::OriginalSshOwner,
    ) -> Result<(), crate::BackupError> {
        let pair = self.original_pair.ok_or_else(original_pipe_error)?;
        let OriginalPipeOrigin::Source(original) = pair.origin else {
            return Err(original_pipe_error());
        };
        original.require_negative_m6_custody(ssh)?;
        // Actual worker/read/write is naturally returned before registrations
        // and the one original write FD are closed, never while in flight.
        drop(self.output.take());
        let mut slot = pair.output.fd.lock().map_err(|_| original_pipe_error())?;
        drop(slot.take());
        self.syntax.poison();
        Ok(())
    }
    pub(super) fn prepay_original_serialization(
        &self,
        body_bytes: usize,
    ) -> Result<(), crate::BackupError> {
        let work = (body_bytes as u64)
            .checked_mul(4)
            .and_then(|n| n.checked_add(profile::CONTROL_BYTES.checked_mul(64)?))
            .ok_or_else(original_pipe_error)?;
        self.clock
            .prepay(work, 32)
            .map_err(|_| original_pipe_error())
    }
    pub(super) fn syntax_original(&self) -> &WireSyntax {
        &self.syntax
    }
    pub(super) fn raw_inbound(&self) -> &[u8] {
        &self.buffer
    }
}

#[cfg(test)]
mod tests;

//! Actual ordinary-UID Linux anonymous pipes. These are NOT SSH/Child/auth tests.
use super::super::{encode_frame, tests as fixture};
use super::*;
use std::os::fd::FromRawFd;

fn pipe_with(flags: i32) -> (OwnedFd, OwnedFd) {
    assert_ne!(
        unsafe { libc::geteuid() },
        0,
        "TASK2B_NATIVE_REQUIRES_ORDINARY_UID"
    );
    let mut fds = [-1; 2];
    assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), flags) }, 0);
    unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
}
fn pipe() -> (OwnedFd, OwnedFd) {
    pipe_with(libc::O_NONBLOCK | libc::O_CLOEXEC)
}
fn require_sigpipe_precondition() {
    assert!(
        sigpipe_ignored().is_ok(),
        "TASK2B_NATIVE_NOT_EXECUTED_SIGPIPE_PRECONDITION_UNSATISFIED; NO_SIGNAL_CHANGE"
    );
}
fn owner() -> (PipeIo, OwnedFd, OwnedFd) {
    owner_for(Direction::DestinationToSource)
}
fn owner_for(outbound: Direction) -> (PipeIo, OwnedFd, OwnedFd) {
    require_sigpipe_precondition();
    let (read, peer_write) = pipe();
    let (peer_read, write) = pipe();
    (
        PipeIo::new(read, write, outbound).unwrap(),
        peer_write,
        peer_read,
    )
}
fn queued(fd: &AsyncFd<OwnedFd>) -> i32 {
    let mut count = -1;
    assert_eq!(
        unsafe { libc::ioctl(fd.get_ref().as_raw_fd(), libc::FIONREAD, &mut count) },
        0
    );
    count
}
fn small_write(fd: &OwnedFd, bytes: &[u8]) {
    require_sigpipe_precondition();
    assert_eq!(
        unsafe { libc::write(fd.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) },
        bytes.len() as isize
    );
}
fn fill_pipe(fd: &AsyncFd<OwnedFd>) -> usize {
    require_sigpipe_precondition();
    let block = [0_u8; 4096];
    let mut bytes = 0;
    loop {
        let n =
            unsafe { libc::write(fd.get_ref().as_raw_fd(), block.as_ptr().cast(), block.len()) };
        if n < 0 {
            assert_eq!(io::Error::last_os_error().kind(), io::ErrorKind::WouldBlock);
            return bytes;
        }
        assert!(n > 0);
        bytes += n as usize;
    }
}
fn past_end_fixture(io: &mut PipeIo) {
    // Hypothetical syntax DATA state only; real EOF/budget below use actual FDs.
    // The full native end-order test separately sends every fixture frame.
    io.syntax.phase = SyntaxPhase::BothEndsSeen;
    io.inbound_end = true;
    io.outbound_end = true;
}

#[tokio::test(flavor = "current_thread")]
async fn header_budget_room_is_checked_before_native_read() {
    let (mut io, peer, _output) = owner();
    let raw = fixture::hello(Direction::SourceToDestination, fixture::source());
    small_write(&peer, &raw[..HEADER]);
    fixture::fill_wire(
        &mut io.syntax.budget,
        profile::WIRE_BYTES - (profile::HEADER_BYTES - 1),
    );
    assert_eq!(queued(&io.input), 20);
    assert_eq!(
        io.read_frame().await.err(),
        Some(PipeError::Wire(WireError::Budget))
    );
    assert_eq!(queued(&io.input), 20);
    assert_eq!(io.read_progress, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn payload_reservation_precedes_allocation_and_native_payload_read() {
    let (mut io, peer, _output) = owner();
    let raw = fixture::hello(Direction::SourceToDestination, fixture::source());
    small_write(&peer, &raw);
    fixture::fill_wire(
        &mut io.syntax.budget,
        profile::WIRE_BYTES - raw.len() as u64 + 1,
    );
    assert_eq!(
        io.read_frame().await.err(),
        Some(PipeError::Wire(WireError::Budget))
    );
    assert_eq!(io.read_progress, HEADER);
    assert_eq!(queued(&io.input) as usize, raw.len() - HEADER);
    assert!(io.buffer.is_empty());
    assert_eq!(io.syntax.phase, SyntaxPhase::Poisoned);
}

#[tokio::test(flavor = "current_thread")]
async fn fragmented_native_reads_reassemble_one_exact_borrowed_frame() {
    let (mut io, peer, _output) = owner();
    let raw = fixture::hello(Direction::SourceToDestination, fixture::source());
    let expected = raw.clone();
    let writer = tokio::spawn(async move {
        let fd = AsyncFd::new(peer).unwrap();
        let mut clock = SessionClock::start();
        for bytes in raw.chunks(7) {
            let mut progress = 0;
            write_all(&fd, &mut clock, bytes, &mut progress)
                .await
                .unwrap();
            tokio::task::yield_now().await;
        }
    });
    assert_eq!(io.read_frame().await.unwrap().header.kind, Kind::Metadata);
    assert_eq!(io.read_progress, expected.len());
    assert_eq!(io.buffer, expected);
    writer.await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn native_partial_header_and_payload_eof_are_terminal() {
    for bytes in [5, HEADER + 3] {
        let (mut io, peer, _output) = owner();
        let raw = fixture::hello(Direction::SourceToDestination, fixture::source());
        small_write(&peer, &raw[..bytes]);
        drop(peer);
        assert_eq!(io.read_frame().await.err(), Some(PipeError::UnexpectedEof));
        assert_eq!(io.read_progress, bytes);
        assert_eq!(io.read_frame().await.err(), Some(PipeError::Poisoned));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn native_blocked_write_uses_original_absolute_deadline_and_retains_buffer() {
    let (mut io, _peer, _output) = owner();
    io.syntax
        .observe_untrusted(&fixture::hello(
            Direction::SourceToDestination,
            fixture::source(),
        ))
        .unwrap();
    let raw = fixture::hello(Direction::DestinationToSource, fixture::source());
    assert!(fill_pipe(io.output.as_ref().unwrap()) > 0);
    io.clock.idle = Instant::now() + Duration::from_millis(150);
    assert_eq!(
        io.write_frame(&raw).await.err(),
        Some(PipeError::IdleDeadline)
    );
    assert_eq!(io.write_progress, 0);
    assert_eq!(io.buffer, raw);
    assert_eq!(io.syntax.phase, SyntaxPhase::Poisoned);
}

#[tokio::test(flavor = "current_thread")]
async fn native_short_writes_complete_only_after_peer_drain() {
    let (mut io, _peer, output) = owner_for(Direction::SourceToDestination);
    let (syntax, raw) = fixture::large_chunk_fixture();
    io.syntax = syntax;
    let expected = raw.clone();
    assert!(raw.len() > 4096);
    let padding = fill_pipe(io.output.as_ref().unwrap());
    assert!(padding >= 4096);
    let reader = tokio::spawn(async move {
        let fd = AsyncFd::new(output).unwrap();
        let mut clock = SessionClock::start();
        tokio::time::sleep(Duration::from_millis(20)).await;
        let mut initial = [0_u8; 4096];
        assert_eq!(
            read_some(&fd, &mut clock, &mut initial).await.unwrap(),
            4096
        );
        // Only 4096 room is made, less than this frame. Observe the kernel
        // queue refill BEFORE draining again, proving a positive short write.
        tokio::time::timeout(Duration::from_secs(2), async {
            while queued(&fd) != padding as i32 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let mut captured = Vec::new();
        let mut total = 4096;
        let mut block = [0_u8; 31];
        while total < padding + expected.len() {
            let n = read_some(&fd, &mut clock, &mut block).await.unwrap();
            assert!(n > 0);
            for (i, byte) in block[..n].iter().enumerate() {
                if total + i >= padding {
                    captured.push(*byte);
                }
            }
            total += n;
        }
        assert_eq!(captured, expected);
    });
    io.write_frame(&raw).await.unwrap();
    assert_eq!(io.write_progress, raw.len());
    reader.await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn cancel_during_partial_read_poison_preserves_native_offsets() {
    for bytes in [5, HEADER + 3] {
        let (mut io, peer, _output) = owner();
        let raw = fixture::hello(Direction::SourceToDestination, fixture::source());
        small_write(&peer, &raw[..bytes]);
        let mut future = Box::pin(io.read_frame());
        tokio::select! {
            result = &mut future => panic!("unexpected complete read: {:?}", result.err()),
            _ = tokio::time::sleep(Duration::from_millis(30)) => {}
        }
        drop(future);
        assert_eq!(io.read_progress, bytes);
        assert_eq!(io.syntax.phase, SyntaxPhase::Poisoned);
        assert_eq!(io.read_frame().await.err(), Some(PipeError::Poisoned));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cancel_during_blocked_write_is_sticky_without_replay() {
    let (mut io, _peer, _output) = owner();
    io.syntax
        .observe_untrusted(&fixture::hello(
            Direction::SourceToDestination,
            fixture::source(),
        ))
        .unwrap();
    let raw = fixture::hello(Direction::DestinationToSource, fixture::source());
    assert!(fill_pipe(io.output.as_ref().unwrap()) > 0);
    let mut future = Box::pin(io.write_frame(&raw));
    tokio::select! {
        result = &mut future => panic!("unexpected complete write: {:?}", result.err()),
        _ = tokio::time::sleep(Duration::from_millis(30)) => {}
    }
    drop(future);
    assert_eq!(io.buffer, raw);
    assert_eq!(io.syntax.phase, SyntaxPhase::Poisoned);
    assert_eq!(io.write_frame(&raw).await.err(), Some(PipeError::Poisoned));
}

#[tokio::test(flavor = "current_thread")]
async fn native_operation_idle_and_handshake_clocks_are_not_per_call_timeouts() {
    for operation in [true, false] {
        let (mut io, _peer, _output) = owner();
        if operation {
            io.clock.operation = Instant::now() + Duration::from_millis(100);
        } else {
            io.clock.idle = Instant::now() + Duration::from_millis(100);
        }
        let expected = if operation {
            PipeError::OperationDeadline
        } else {
            PipeError::IdleDeadline
        };
        assert_eq!(io.read_frame().await.err(), Some(expected));
        assert_eq!(io.syntax.phase, SyntaxPhase::Poisoned);
    }
    let (mut io, peer, _output) = owner();
    io.clock.handshake = Instant::now() + Duration::from_millis(150);
    let operation = io.clock.operation;
    let handshake = io.clock.handshake;
    let writer = tokio::spawn(async move {
        for _ in 0..3 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            small_write(&peer, b"K");
        }
        tokio::time::sleep(Duration::from_millis(180)).await;
    });
    assert_eq!(
        io.read_frame().await.err(),
        Some(PipeError::HandshakeDeadline)
    );
    assert_eq!(io.clock.operation, operation);
    assert_eq!(io.clock.handshake, handshake);
    assert!(io.read_progress > 0);
    writer.await.unwrap();
}

#[test]
fn interrupted_classification_is_not_a_native_signal_experiment() {
    let mut clock = SessionClock::start();
    let before = (clock.operation, clock.handshake, clock.idle);
    for error in [io::ErrorKind::Interrupted, io::ErrorKind::WouldBlock] {
        assert!(retry_without_progress(error));
        clock.check().unwrap();
        assert_eq!((clock.operation, clock.handshake, clock.idle), before);
    }
    assert!(!retry_without_progress(io::ErrorKind::BrokenPipe));
    clock.progress(0).unwrap();
    assert_eq!(clock.idle, before.2);
    clock.progress(1).unwrap();
    assert_eq!(clock.operation, before.0);
    assert_eq!(clock.handshake, before.1);
}

#[tokio::test(flavor = "current_thread")]
async fn native_eof_probe_prepays_one_byte_and_refuses_without_room() {
    let (mut io, peer, _output) = owner();
    past_end_fixture(&mut io);
    fixture::fill_wire(&mut io.syntax.budget, profile::WIRE_BYTES);
    small_write(&peer, b"x");
    assert_eq!(
        io.expect_eof().await.err(),
        Some(PipeError::Wire(WireError::Budget))
    );
    assert_eq!(queued(&io.input), 1);
    assert!(!io.eof_probe_started);
    assert!(!io.eof_observed);

    let (mut io, peer, _output) = owner();
    past_end_fixture(&mut io);
    fixture::fill_wire(&mut io.syntax.budget, profile::WIRE_BYTES - 1);
    drop(peer);
    io.expect_eof().await.unwrap();
    assert!(io.eof_observed);
    assert!(io.syntax.budget.reserve_eof_probe().is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn native_eof_read_zero_is_not_refunded_and_no_second_probe_is_allowed() {
    let (mut io, peer, _output) = owner();
    past_end_fixture(&mut io);
    fixture::fill_wire(
        &mut io.syntax.budget,
        profile::WIRE_BYTES - profile::HEADER_BYTES,
    );
    drop(peer);
    io.expect_eof().await.unwrap();
    // 20 room became 19 even though actual EOF returned zero: no refund.
    assert!(
        io.syntax
            .budget
            .charge_frame(profile::FrameKind::End, 0)
            .is_err()
    );
    assert!(io.expect_eof().await.is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn native_eof_probe_waits_for_kernel_zero_and_rejects_extra_one_byte() {
    let (mut io, _peer, _output) = owner();
    past_end_fixture(&mut io);
    io.clock.idle = Instant::now() + Duration::from_millis(100);
    assert_eq!(io.expect_eof().await.err(), Some(PipeError::IdleDeadline));
    assert!(!io.eof_observed);
    let (mut io, peer, _output) = owner();
    past_end_fixture(&mut io);
    small_write(&peer, b"xy");
    assert_eq!(io.expect_eof().await.err(), Some(PipeError::TrailingData));
    assert_eq!(queued(&io.input), 1);
    assert_eq!(io.expect_eof().await.err(), Some(PipeError::Poisoned));
    assert_eq!(queued(&io.input), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn native_two_pipe_end_halfclose_order_is_only_untrusted_transport() {
    require_sigpipe_precondition();
    let (d_input, s_output) = pipe();
    let (s_input, d_output) = pipe();
    let mut s = PipeIo::new(s_input, s_output, Direction::SourceToDestination).unwrap();
    let mut d = PipeIo::new(d_input, d_output, Direction::DestinationToSource).unwrap();
    for raw in fixture::syntax_fixture_frames() {
        let header = inspect_header(&raw[..HEADER]).unwrap();
        if header.direction == Direction::SourceToDestination {
            let (written, read) = tokio::join!(s.write_frame(&raw), d.read_frame());
            written.unwrap();
            assert_eq!(read.unwrap().header, header);
            if header.kind == Kind::End {
                s.close_output_after_end().unwrap();
                d.expect_eof().await.unwrap();
            }
        } else {
            let (written, read) = tokio::join!(d.write_frame(&raw), s.read_frame());
            written.unwrap();
            assert_eq!(read.unwrap().header, header);
            if header.kind == Kind::End {
                d.close_output_after_end().unwrap();
                s.expect_eof().await.unwrap();
            }
        }
    }
    assert_eq!(
        s.finish_untrusted().unwrap(),
        UntrustedPipeTermination::EndFramesAndObservedEof
    );
    assert_eq!(
        d.finish_untrusted().unwrap(),
        UntrustedPipeTermination::EndFramesAndObservedEof
    );
    // Opaque fixture receipt/signatures were NEVER verified. No authority exists.
}

#[tokio::test(flavor = "current_thread")]
async fn native_abort_and_eof_never_mean_normal_completion() {
    let (mut io, peer, _output) = owner();
    let raw = encode_frame(Kind::Abort, Direction::SourceToDestination, 0, &[]).unwrap();
    small_write(&peer, &raw);
    io.read_frame().await.unwrap();
    io.close_output_after_end().unwrap();
    drop(peer);
    io.expect_eof().await.unwrap();
    assert_eq!(
        io.finish_untrusted().unwrap(),
        UntrustedPipeTermination::AbortAndObservedEof
    );
}

#[tokio::test(flavor = "current_thread")]
async fn native_fifo_access_flags_and_aliases_are_checked_without_mutation() {
    require_sigpipe_precondition();
    let (read, write) = pipe();
    assert_eq!(
        PipeIo::new(read, write, Direction::SourceToDestination).err(),
        Some(PipeError::UnsupportedDescriptor)
    );
    let (read, _peer) = pipe_with(libc::O_CLOEXEC);
    let (_other, write) = pipe();
    assert_eq!(
        PipeIo::new(read, write, Direction::SourceToDestination).err(),
        Some(PipeError::UnsupportedDescriptor)
    );
    let (_read, wrong_reader) = pipe();
    let (_other, write) = pipe();
    assert_eq!(
        PipeIo::new(wrong_reader, write, Direction::SourceToDestination).err(),
        Some(PipeError::UnsupportedDescriptor)
    );
    let raw = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    assert!(raw >= 0);
    let nonpipe = unsafe { OwnedFd::from_raw_fd(raw) };
    let (_other, write) = pipe();
    assert_eq!(
        PipeIo::new(nonpipe, write, Direction::SourceToDestination).err(),
        Some(PipeError::UnsupportedDescriptor)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn native_broken_pipe_reports_actual_epipe_under_sigpipe_precondition() {
    let (mut io, _peer, output) = owner();
    io.syntax
        .observe_untrusted(&fixture::hello(
            Direction::SourceToDestination,
            fixture::source(),
        ))
        .unwrap();
    drop(output);
    let raw = fixture::hello(Direction::DestinationToSource, fixture::source());
    assert_eq!(
        io.write_frame(&raw).await.err(),
        Some(PipeError::BrokenPipe)
    );
    assert_eq!(io.syntax.phase, SyntaxPhase::Poisoned);
}

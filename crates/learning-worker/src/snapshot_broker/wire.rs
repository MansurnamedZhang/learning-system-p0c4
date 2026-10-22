//! Bounded Linux stream frames. Every receive checks ancillary data, including
//! bytes after the JSON body; a descriptor must never become a broker input.
use super::{FRAME_CAP, invalid};
use std::{
    fs::File,
    io::{self, Write},
    mem,
    net::Shutdown,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, net::UnixStream},
    },
    path::Path,
    time::{Duration, Instant},
};

const DEADLINE: Duration = Duration::from_secs(2);

pub(super) fn read_start_gate(fd: libc::c_int, wait: Duration) -> io::Result<()> {
    if wait.is_zero() || wait > Duration::from_secs(5) {
        return Err(invalid());
    }
    let deadline = Instant::now() + wait;
    let mut raw = [0u8; 7];
    let mut used = 0;
    loop {
        let left = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(invalid)?;
        let millis = left.as_millis().clamp(1, i32::MAX as u128) as i32;
        let mut descriptor = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut descriptor, 1, millis) };
        if ready < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if ready == 0 || descriptor.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
            return Err(invalid());
        }
        let read = unsafe { libc::read(fd, raw[used..].as_mut_ptr().cast(), raw.len() - used) };
        if read < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if Instant::now() >= deadline {
            return Err(invalid());
        }
        if read == 0 {
            return if &raw[..used] == b"START\n" && Instant::now() < deadline {
                Ok(())
            } else {
                Err(invalid())
            };
        }
        used += read as usize;
        if used == raw.len() {
            return Err(invalid());
        }
    }
}

#[cfg(test)]
mod start_gate_tests {
    use super::*;
    #[test]
    fn native_start_gate_waits_for_fixed_marker_and_eof_before_any_signal() {
        let (mut controller, client) = UnixStream::pair().unwrap();
        let (done, result) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            let value = read_start_gate(client.as_raw_fd(), Duration::from_secs(1));
            done.send(value.is_ok()).unwrap();
        });
        assert!(result.recv_timeout(Duration::from_millis(20)).is_err());
        controller.write_all(b"START\n").unwrap();
        assert!(result.recv_timeout(Duration::from_millis(20)).is_err());
        controller.shutdown(Shutdown::Write).unwrap();
        assert!(result.recv_timeout(Duration::from_secs(1)).unwrap());
        reader.join().unwrap();
    }
    #[test]
    fn native_start_gate_refuses_extra_missing_late_or_unbounded_input() {
        for raw in [b"".as_slice(), b"START", b"START\nextra", b"start\n"] {
            let (mut controller, client) = UnixStream::pair().unwrap();
            controller.write_all(raw).unwrap();
            controller.shutdown(Shutdown::Write).unwrap();
            assert!(read_start_gate(client.as_raw_fd(), Duration::from_millis(100)).is_err());
        }
        let (_controller, client) = UnixStream::pair().unwrap();
        assert!(read_start_gate(client.as_raw_fd(), Duration::from_millis(10)).is_err());
        assert!(read_start_gate(client.as_raw_fd(), Duration::from_secs(6)).is_err());
    }
}

pub(super) fn connect(path: &Path) -> io::Result<UnixStream> {
    let bytes = path.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { mem::zeroed() };
    if bytes.is_empty() || bytes.len() >= address.sun_path.len() || bytes.contains(&0) {
        return Err(invalid());
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    unsafe {
        std::ptr::copy_nonoverlapping(
            bytes.as_ptr(),
            address.sun_path.as_mut_ptr().cast(),
            bytes.len(),
        )
    };
    let fd = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            0,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let stream = unsafe { UnixStream::from_raw_fd(fd) };
    let length = mem::offset_of!(libc::sockaddr_un, sun_path) + bytes.len() + 1;
    if unsafe {
        libc::connect(
            fd,
            (&address as *const libc::sockaddr_un).cast(),
            length as libc::socklen_t,
        )
    } != 0
    {
        // A full listener queue refuses immediately, rather than blocking the
        // worker indefinitely or falling back to its old direct-claim path.
        return Err(io::Error::last_os_error());
    }
    stream.set_nonblocking(false)?;
    Ok(stream)
}

pub(super) fn peer(stream: &UnixStream) -> io::Result<(u32, u32)> {
    let mut credentials: libc::ucred = unsafe { mem::zeroed() };
    let mut size = mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut size,
        )
    };
    if result != 0 || size as usize != mem::size_of::<libc::ucred>() {
        return Err(io::Error::last_os_error());
    }
    Ok((credentials.uid, credentials.gid))
}

fn receive(stream: &UnixStream, bytes: &mut [u8], deadline: Instant) -> io::Result<usize> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "snapshot broker frame deadline"))?;
    stream.set_read_timeout(Some(remaining))?;
    let mut data = libc::iovec {
        iov_base: bytes.as_mut_ptr().cast(),
        iov_len: bytes.len(),
    };
    // usize gives cmsghdr alignment. MSG_CMSG_CLOEXEC protects any received FD
    // before it is immediately closed. Truncated ancillary data also refuses.
    let mut control = [0usize; 32];
    let mut message: libc::msghdr = unsafe { mem::zeroed() };
    message.msg_iov = &mut data;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = mem::size_of_val(&control);
    let count = unsafe { libc::recvmsg(stream.as_raw_fd(), &mut message, libc::MSG_CMSG_CLOEXEC) };
    if count < 0 {
        return Err(io::Error::last_os_error());
    }
    // Always close received rights before rejecting the message, even when
    // MSG_CTRUNC is set. Linux closes rights that did not fit the control area.
    unsafe {
        let mut item = libc::CMSG_FIRSTHDR(&message);
        while !item.is_null() {
            if (*item).cmsg_level == libc::SOL_SOCKET && (*item).cmsg_type == libc::SCM_RIGHTS {
                let header = libc::CMSG_LEN(0) as usize;
                if (*item).cmsg_len >= header {
                    let count = ((*item).cmsg_len - header) / mem::size_of::<libc::c_int>();
                    let descriptors = libc::CMSG_DATA(item).cast::<libc::c_int>();
                    for index in 0..count {
                        drop(File::from_raw_fd(*descriptors.add(index)));
                    }
                }
            }
            item = libc::CMSG_NXTHDR(&message, item);
        }
    }
    if message.msg_controllen != 0 || message.msg_flags & (libc::MSG_CTRUNC | libc::MSG_TRUNC) != 0
    {
        return Err(invalid());
    }
    Ok(count as usize)
}

fn exact(stream: &UnixStream, mut bytes: &mut [u8], deadline: Instant) -> io::Result<()> {
    while !bytes.is_empty() {
        let count = receive(stream, bytes, deadline)?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "incomplete broker frame",
            ));
        }
        bytes = &mut bytes[count..];
    }
    Ok(())
}

pub(super) fn read_frame(stream: &UnixStream) -> io::Result<Vec<u8>> {
    read_bounded(stream, DEADLINE)
}

pub(super) fn read_reply(stream: &UnixStream, wait: Duration) -> io::Result<Vec<u8>> {
    if wait.is_zero() || wait > Duration::from_secs(300) {
        return Err(invalid());
    }
    read_bounded(stream, wait)
}
fn read_bounded(stream: &UnixStream, wait: Duration) -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + wait;
    let mut length = [0u8; 4];
    exact(stream, &mut length, deadline)?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > FRAME_CAP {
        return Err(invalid());
    }
    let mut body = vec![0; length];
    exact(stream, &mut body, deadline)?;
    // One request per connection. Mandatory SHUT_WR prevents hidden second
    // frames or late descriptor delivery from being accepted as a signal.
    if receive(stream, &mut [0], deadline)? != 0 {
        return Err(invalid());
    }
    Ok(body)
}

pub(super) fn write_frame(mut stream: &UnixStream, body: &[u8]) -> io::Result<()> {
    if body.is_empty() || body.len() > FRAME_CAP {
        return Err(invalid());
    }
    stream.set_write_timeout(Some(DEADLINE))?;
    stream.write_all(&(body.len() as u32).to_be_bytes())?;
    stream.write_all(body)?;
    stream.shutdown(Shutdown::Write)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn active_reply_can_wait_past_request_deadline_with_one_absolute_limit() {
        let (one, two) = UnixStream::pair().unwrap();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(2100));
            write_frame(&one, b"redacted scheduling reply").unwrap();
        });
        assert_eq!(
            read_reply(&two, Duration::from_secs(3)).unwrap(),
            b"redacted scheduling reply"
        );
        writer.join().unwrap();
    }
    #[test]
    fn active_reply_wait_refuses_unbounded_or_expired_budget_and_extra_bytes() {
        let (mut one, two) = UnixStream::pair().unwrap();
        assert!(read_reply(&two, Duration::ZERO).is_err());
        assert!(read_reply(&two, Duration::from_secs(301)).is_err());
        assert!(read_reply(&two, Duration::from_millis(10)).is_err());
        one.write_all(&[0, 0, 0, 1, b'x', b'y']).unwrap();
        one.shutdown(Shutdown::Write).unwrap();
        assert!(read_reply(&two, Duration::from_secs(3)).is_err());
    }

    #[test]
    fn real_socket_pair_frame_roundtrip_and_peer_identity() {
        let (one, two) = UnixStream::pair().unwrap();
        write_frame(&one, b"bounded signal").unwrap();
        assert_eq!(read_frame(&two).unwrap(), b"bounded signal");
        assert_eq!(peer(&two).unwrap(), unsafe {
            (libc::geteuid(), libc::getegid())
        });
    }

    #[test]
    fn real_socket_pair_refuses_truncated_oversized_and_extra_frames() {
        for bytes in [
            vec![0, 0],
            (FRAME_CAP as u32 + 1).to_be_bytes().to_vec(),
            vec![0, 0, 0, 1, b'a', b'b'],
        ] {
            let (mut one, two) = UnixStream::pair().unwrap();
            one.write_all(&bytes).unwrap();
            one.shutdown(Shutdown::Write).unwrap();
            assert!(read_frame(&two).is_err());
        }
    }

    #[test]
    fn real_scm_rights_is_rejected_at_header_body_and_trailing_byte() {
        for prefix in [0, 4, 5] {
            let (mut one, two) = UnixStream::pair().unwrap();
            let frame = [0, 0, 0, 1, b'x', b'y'];
            one.write_all(&frame[..prefix]).unwrap();
            let source = File::open("/dev/null").unwrap();
            let mut byte = frame[prefix];
            let mut vector = libc::iovec {
                iov_base: (&mut byte as *mut u8).cast(),
                iov_len: 1,
            };
            let mut control = [0usize; 8];
            let mut message: libc::msghdr = unsafe { mem::zeroed() };
            message.msg_iov = &mut vector;
            message.msg_iovlen = 1;
            message.msg_control = control.as_mut_ptr().cast();
            message.msg_controllen =
                unsafe { libc::CMSG_SPACE(mem::size_of::<libc::c_int>() as u32) } as usize;
            unsafe {
                let item = libc::CMSG_FIRSTHDR(&message);
                (*item).cmsg_level = libc::SOL_SOCKET;
                (*item).cmsg_type = libc::SCM_RIGHTS;
                (*item).cmsg_len = libc::CMSG_LEN(mem::size_of::<libc::c_int>() as u32) as usize;
                std::ptr::write_unaligned(
                    libc::CMSG_DATA(item).cast::<libc::c_int>(),
                    source.as_raw_fd(),
                );
                assert_eq!(
                    libc::sendmsg(one.as_raw_fd(), &message, libc::MSG_NOSIGNAL),
                    1
                );
            }
            one.shutdown(Shutdown::Write).unwrap();
            assert!(read_frame(&two).is_err());
            assert!(source.metadata().is_ok());
        }
    }
}

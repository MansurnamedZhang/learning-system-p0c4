//! The existing S1 frame algorithm, reused by the DB-owned active server.
use std::{
    fs::File,
    io::{self, Write},
    mem,
    net::Shutdown,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::net::UnixStream,
    },
    time::{Duration, Instant},
};
const FRAME_CAP: usize = 4096;
const DEADLINE: Duration = Duration::from_secs(2);
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "snapshot work frame rejected")
}
pub(super) fn peer(stream: &UnixStream) -> io::Result<(u32, u32, i32)> {
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
    Ok((credentials.uid, credentials.gid, credentials.pid))
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
    let deadline = Instant::now() + DEADLINE;
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

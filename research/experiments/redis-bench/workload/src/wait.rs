//! Blocking readiness wait over raw sockets, std only (poll(2) is declared
//! directly; std already links libc). Lets a worker sleep in the kernel
//! instead of spinning, so client CPU time reflects work done.
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::time::Duration;

#[repr(C)]
struct PollFd { fd: i32, events: i16, revents: i16 }
const POLLIN: i16 = 0x001;
const POLLOUT: i16 = 0x004;

#[cfg(target_os = "linux")]
type NFds = std::ffi::c_ulong;
#[cfg(not(target_os = "linux"))]
type NFds = std::ffi::c_uint;

extern "C" {
    fn poll(fds: *mut PollFd, nfds: NFds, timeout: i32) -> i32;
}

/// A socket and whether the caller also waits for it to become writable
/// (readability is always awaited).
pub struct Interest { pub fd: RawFd, pub write: bool }

/// Blocks until any interest is ready or `timeout` elapses (rounded up to a
/// whole millisecond, at least 1 ms, so it never degenerates into a spin).
/// Interruption by a signal returns normally; the caller rescans anyway.
pub fn wait(interests: &[Interest], timeout: Duration) -> io::Result<()> {
    let mut fds: Vec<PollFd> = interests.iter().map(|i| PollFd {
        fd: i.fd, events: POLLIN | if i.write { POLLOUT } else { 0 }, revents: 0,
    }).collect();
    let ms = timeout.as_nanos().div_ceil(1_000_000).clamp(1, i32::MAX as u128) as i32;
    // SAFETY: fds is a live, correctly laid out array of fds.len() entries.
    let n = unsafe { poll(fds.as_mut_ptr(), fds.len() as NFds, ms) };
    if n >= 0 { return Ok(()); }
    let e = io::Error::last_os_error();
    if e.kind() == io::ErrorKind::Interrupted { Ok(()) } else { Err(e) }
}

/// Blocks until `socket` is readable (or hung up) or `timeout` elapses.
#[allow(dead_code)]
pub fn wait_readable(socket: &impl AsRawFd, timeout: Duration) -> io::Result<()> {
    wait(&[Interest { fd: socket.as_raw_fd(), write: false }], timeout)
}

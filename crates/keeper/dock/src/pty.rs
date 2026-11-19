//! The system calls that make a pseudo-terminal and the pipe that wakes its threads.
//!
//! Every descriptor made here is close-on-exec from its birth: other threads of the
//! Keeper start programs at any moment, and none of them may take a terminal along.

use std::ffi::CString;
use std::io;
use std::os::fd::{FromRawFd, OwnedFd, RawFd};

use crate::Size;

/// The most characters a slave's name has, with the end of the string.
const SLAVE_NAME_LENGTH: usize = 128;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    /// macOS has had this since 10.13.4, and the `libc` crate does not declare it there.
    fn ptsname_r(fd: libc::c_int, buf: *mut libc::c_char, buflen: libc::size_t) -> libc::c_int;
}

#[cfg(not(target_os = "macos"))]
use libc::ptsname_r;

/// Turns the return of a system call into an error when it failed.
pub(crate) fn checked(result: libc::c_int) -> io::Result<libc::c_int> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result)
    }
}

/// A new master and the slave that belongs to it, both of `size`.
pub(crate) fn open_pair(size: Size) -> io::Result<(OwnedFd, OwnedFd)> {
    // SAFETY: the path is a C string that outlives the call, and the flags are the
    // system's own. The descriptor is wrapped at once, so it is closed on every path out.
    let master = unsafe {
        let fd = checked(libc::open(
            c"/dev/ptmx".as_ptr(),
            libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC,
        ))?;
        OwnedFd::from_raw_fd(fd)
    };
    let raw = std::os::fd::AsRawFd::as_raw_fd(&master);
    // SAFETY: `raw` is open for as long as `master` is.
    unsafe {
        checked(libc::grantpt(raw))?;
        checked(libc::unlockpt(raw))?;
    }
    let mut name = [0 as libc::c_char; SLAVE_NAME_LENGTH];
    // SAFETY: the buffer is as long as the length says.
    let rc = unsafe { ptsname_r(raw, name.as_mut_ptr(), name.len()) };
    if rc != 0 {
        return Err(if rc == -1 {
            io::Error::last_os_error()
        } else {
            io::Error::from_raw_os_error(rc)
        });
    }
    // SAFETY: `ptsname_r` wrote a string with its end into the buffer.
    let name = unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) }.to_owned();

    let slave = open_slave(&name)?;
    // The size goes in before the program starts, so that it never sees another. Not
    // before the slave is open: macOS refuses it on a master that has no slave.
    set_size(raw, size)?;
    Ok((master, slave))
}

fn open_slave(name: &CString) -> io::Result<OwnedFd> {
    // SAFETY: as for the master.
    unsafe {
        let fd = checked(libc::open(
            name.as_ptr(),
            libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC,
        ))?;
        Ok(OwnedFd::from_raw_fd(fd))
    }
}

/// Gives the terminal a size; the system tells the program with a signal when it changes.
pub(crate) fn set_size(master: RawFd, size: Size) -> io::Result<()> {
    let window = libc::winsize {
        ws_row: size.rows.max(1),
        ws_col: size.columns.max(1),
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: the pointer is to a value that outlives the call, of the type the request
    // takes.
    checked(unsafe { libc::ioctl(master, libc::TIOCSWINSZ as _, &window) }).map(|_| ())
}

/// Makes the descriptor refuse a blocking wait: reads and writes on it return at once
/// when they cannot go on, and the threads wait with `poll` instead, which they can
/// leave when told to.
pub(crate) fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    // SAFETY: plain flag calls on an open descriptor.
    unsafe {
        let flags = checked(libc::fcntl(fd, libc::F_GETFL))?;
        checked(libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK))?;
    }
    Ok(())
}

/// A pipe of which both ends are close-on-exec: the read end and the write end.
pub(crate) fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut ends = [0 as libc::c_int; 2];
    #[cfg(not(target_os = "macos"))]
    // SAFETY: the array holds the two descriptors the call writes.
    checked(unsafe { libc::pipe2(ends.as_mut_ptr(), libc::O_CLOEXEC) })?;
    // macOS has no `pipe2`. The window between the two calls is why the program's start
    // marks every descriptor close-on-exec there.
    #[cfg(target_os = "macos")]
    // SAFETY: as above, and the flags are set on descriptors that are open.
    unsafe {
        checked(libc::pipe(ends.as_mut_ptr()))?;
        for end in ends {
            libc::fcntl(end, libc::F_SETFD, libc::FD_CLOEXEC);
        }
    }
    // SAFETY: the two descriptors were just made and nothing else owns them.
    Ok(unsafe { (OwnedFd::from_raw_fd(ends[0]), OwnedFd::from_raw_fd(ends[1])) })
}

/// Waits for the master to be ready for `events`, or to fail, and for the pipe that says
/// the program was collected. Gives whether the master is ready, and whether the pipe
/// spoke. `timeout` is in milliseconds, `None` for no end. A pipe already known to have
/// spoken is left out with `wake: None`. An interrupted wait gives `(false, false)`.
pub(crate) fn wait(
    master: RawFd,
    events: libc::c_short,
    wake: Option<RawFd>,
    timeout: Option<u64>,
) -> io::Result<(bool, bool)> {
    let mut fds = [
        libc::pollfd {
            fd: master,
            events,
            revents: 0,
        },
        libc::pollfd {
            fd: wake.unwrap_or(-1),
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    let timeout = timeout.map_or(-1, |ms| ms.min(i32::MAX as u64) as libc::c_int);
    // SAFETY: the array is as long as the count says; a descriptor of -1 is skipped.
    let rc = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, timeout) };
    if rc < 0 {
        let error = io::Error::last_os_error();
        return if error.kind() == io::ErrorKind::Interrupted {
            Ok((false, false))
        } else {
            Err(error)
        };
    }
    // A closed or failed master counts as ready: the read or the write then says why.
    let failed = libc::POLLHUP | libc::POLLERR | libc::POLLNVAL;
    Ok((fds[0].revents & (events | failed) != 0, fds[1].revents != 0))
}

/// Reads what the descriptor holds. `Ok(None)` when there is nothing now, or the call
/// was interrupted; `Ok(Some(0))` at the end of the file.
pub(crate) fn read(fd: RawFd, buffer: &mut [u8]) -> io::Result<Option<usize>> {
    // SAFETY: the pointer and the length are those of the buffer.
    let count = unsafe { libc::read(fd, buffer.as_mut_ptr().cast(), buffer.len()) };
    if count >= 0 {
        return Ok(Some(count as usize));
    }
    let error = io::Error::last_os_error();
    match error.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted => Ok(None),
        _ => Err(error),
    }
}

/// Writes what the descriptor takes now. `Ok(None)` when it takes nothing now, or the call
/// was interrupted.
pub(crate) fn write(fd: RawFd, bytes: &[u8]) -> io::Result<Option<usize>> {
    // SAFETY: the pointer and the length are those of the slice.
    let count = unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
    if count >= 0 {
        return Ok(Some(count as usize));
    }
    let error = io::Error::last_os_error();
    match error.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted => Ok(None),
        _ => Err(error),
    }
}

/// The descriptors a program must not hold on macOS, where the standard library makes
/// some and only then marks them close-on-exec: how many to mark, from 3 up.
#[cfg(target_os = "macos")]
pub(crate) fn descriptor_limit() -> libc::c_int {
    const CAP: libc::rlim_t = 65_536;
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: the pointer is to a value the call fills.
    let found = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } == 0;
    if found {
        limit.rlim_cur.min(CAP) as libc::c_int
    } else {
        CAP as libc::c_int
    }
}

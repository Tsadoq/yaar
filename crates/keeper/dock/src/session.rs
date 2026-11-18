//! Which processes belong to the session a terminal's program leads.

use libc::pid_t;

/// When a process started, as the system tells it: two numbers that are the same for one
/// process as long as it lives, and differ between two processes that had the same id one
/// after the other.
type Started = (u64, u64);

/// A session that still held a process when it was noted, and which processes those were.
///
/// A session's id is its leader's id. While a process is in the session the system gives
/// that id to no other, but once the last one is gone it may, and any process that then
/// starts a session of its own leads one with the same id. A process's start time tells
/// the two apart: the session is the old one only while one of the processes noted for it
/// lives.
#[derive(Clone, Debug)]
pub(crate) struct Remembered {
    session: pid_t,
    members: Vec<(pid_t, Started)>,
}

impl Remembered {
    /// The session of a program that has been collected, if a process of it is left.
    pub(crate) fn of(session: pid_t) -> Option<Remembered> {
        let members = noted(session);
        (!members.is_empty()).then_some(Remembered { session, members })
    }

    /// The session's id.
    pub(crate) fn session(&self) -> pid_t {
        self.session
    }

    /// The same session as it is now, when it is still the one noted: a process noted
    /// for it is still alive. `None` when it is empty, or only another session by the
    /// same id.
    pub(crate) fn renewed(&self) -> Option<Remembered> {
        let members = noted(self.session);
        members
            .iter()
            .any(|member| self.members.contains(member))
            .then_some(Remembered {
                session: self.session,
                members,
            })
    }
}

/// The live processes of one session with their start times.
fn noted(session: pid_t) -> Vec<(pid_t, Started)> {
    members(&[session])
        .into_iter()
        .filter_map(|pid| Some((pid, probe(pid)?.started)))
        .collect()
}

/// Every live process whose session is one of `sessions`: a program that was started
/// with `setsid` is the leader, and its id is the session's. A process that is only
/// waiting to be collected is not live. A process that started a session of its own is
/// not in the list.
pub(crate) fn members(sessions: &[pid_t]) -> Vec<pid_t> {
    if sessions.is_empty() {
        return Vec::new();
    }
    all_processes()
        .into_iter()
        .filter(|&pid| pid > 0)
        // SAFETY: `getsid` only reads; a process that is gone answers with an error,
        // which is not a session of ours.
        .filter(|&pid| sessions.contains(&unsafe { libc::getsid(pid) }))
        .filter(|&pid| probe(pid).is_some_and(|probe| !probe.zombie))
        .collect()
}

/// What the system says of one process.
struct Probe {
    /// It has ended and waits to be collected.
    zombie: bool,
    started: Started,
}

#[cfg(not(target_os = "macos"))]
fn all_processes() -> Vec<pid_t> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
        .collect()
}

/// `None` when the process is gone. The state is the character after the name, which is in
/// brackets and may hold any character itself; the start time is the 22nd field of the
/// line, counting the pid as the first.
#[cfg(not(target_os = "macos"))]
fn probe(pid: pid_t) -> Option<Probe> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, rest) = stat.rsplit_once(')')?;
    let mut fields = rest.split_whitespace();
    let zombie = fields.next()?.starts_with('Z');
    // The state is the 3rd field and the start time the 22nd.
    let started = fields.nth(18)?.parse().ok()?;
    Some(Probe {
        zombie,
        started: (started, 0),
    })
}

#[cfg(target_os = "macos")]
fn all_processes() -> Vec<pid_t> {
    // SAFETY: a null buffer asks for the count.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return Vec::new();
    }
    // Room for processes that start between the two calls.
    let mut pids = vec![0 as pid_t; count as usize + 64];
    let size = (pids.len() * size_of::<pid_t>()) as libc::c_int;
    // SAFETY: the buffer is as large as the size says.
    let found = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), size) };
    pids.truncate(found.max(0) as usize);
    pids
}

/// `None` when the process is gone.
#[cfg(target_os = "macos")]
fn probe(pid: pid_t) -> Option<Probe> {
    // SAFETY: an all-zero value is a valid `proc_bsdinfo`, and the call fills it up to
    // the size given.
    unsafe {
        let mut info: libc::proc_bsdinfo = std::mem::zeroed();
        let size = size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let read = libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            std::ptr::from_mut(&mut info).cast(),
            size,
        );
        // Nothing read: it went between the listing and now.
        (read == size).then_some(Probe {
            zombie: info.pbi_status == libc::SZOMB,
            started: (info.pbi_start_tvsec, info.pbi_start_tvusec),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command};

    use super::*;

    /// A session leader that is killed and collected when the test ends, however it ends.
    struct Leader(Child);

    impl Drop for Leader {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// A process that leads a session of its own.
    fn leader() -> Leader {
        let mut command = Command::new("sleep");
        command.arg("300");
        // SAFETY: `setsid` is safe between fork and exec.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Leader(command.spawn().expect("start a session leader"))
    }

    #[test]
    fn a_session_is_kept_while_a_process_noted_for_it_lives_and_not_after() {
        let mut child = leader();
        let session = child.0.id() as pid_t;
        let noted = Remembered::of(session).expect("the leader is in its session");
        assert_eq!(noted.session(), session);
        let again = noted.renewed().expect("still there");
        assert_eq!(again.members, noted.members);

        child.0.kill().expect("kill the leader");
        child.0.wait().expect("collect the leader");
        assert!(noted.renewed().is_none());
        assert!(Remembered::of(session).is_none());
    }

    #[test]
    fn a_session_by_the_same_id_that_began_later_is_not_the_one_that_was_noted() {
        let mut child = leader();
        let session = child.0.id() as pid_t;
        // What the system would show after the noted session emptied and its id went to
        // another process that began a session: the id is the same and so is the member's
        // id, but the process started at another time.
        let mut noted = Remembered::of(session).expect("the leader is in its session");
        for (_, started) in &mut noted.members {
            started.0 += 1;
        }
        assert!(noted.renewed().is_none());
        child.0.kill().expect("kill the leader");
        child.0.wait().expect("collect the leader");
    }

    #[test]
    fn a_process_that_has_ended_is_not_a_member() {
        let mut child = leader();
        let session = child.0.id() as pid_t;
        assert_eq!(members(&[session]), vec![session]);
        child.0.kill().expect("kill the leader");
        child.0.wait().expect("collect the leader");
        assert!(members(&[session]).is_empty());
        assert!(members(&[]).is_empty());
    }
}

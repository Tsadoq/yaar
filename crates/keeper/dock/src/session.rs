//! Which processes belong to the session a terminal's program leads.

use libc::pid_t;

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
        .filter(|&pid| !is_zombie(pid))
        .collect()
}

/// The ones of `sessions` that still have a live process.
pub(crate) fn occupied(sessions: &[pid_t]) -> Vec<pid_t> {
    let mut found: Vec<pid_t> = members(sessions)
        .into_iter()
        // SAFETY: as above.
        .map(|pid| unsafe { libc::getsid(pid) })
        .filter(|session| sessions.contains(session))
        .collect();
    found.sort_unstable();
    found.dedup();
    found
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

/// The state is the character after the name, which is in brackets and may hold any
/// character itself.
#[cfg(not(target_os = "macos"))]
fn is_zombie(pid: pid_t) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        // It went between the listing and now.
        return true;
    };
    stat.rsplit_once(')')
        .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'))
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

#[cfg(target_os = "macos")]
fn is_zombie(pid: pid_t) -> bool {
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
        read != size || info.pbi_status == libc::SZOMB
    }
}

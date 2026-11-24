//! The user's login shell, run once for its environment, for a program the system started with its
//! own short search path (Finder, Dock, desktop menu).

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::Read;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

pub const SHELL_DEADLINE: Duration = Duration::from_secs(10);

/// Printed before the variables: start-up files may print too, only what follows the last marker
/// counts.
pub const MARKER: &str = "yaar-environment-follows";

pub const COMMAND: &str = "printf '%s' 'yaar-environment-follows'; exec env -0";

/// Variables that describe the shell's own run, not the user's setup: never taken from the shell.
const NOT_TAKEN: [&str; 4] = ["PWD", "OLDPWD", "SHLVL", "_"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginShell {
    pub program: PathBuf,
    pub deadline: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellError {
    Start(String),
    Status(Option<i32>),
    NoMarker,
    Deadline(Duration),
}

impl fmt::Display for ShellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start(why) => write!(f, "it could not be started: {why}"),
            Self::Status(Some(code)) => write!(f, "it ended with status {code}"),
            Self::Status(None) => write!(f, "it was ended by a signal"),
            Self::NoMarker => write!(f, "it printed no environment"),
            Self::Deadline(within) => write!(f, "it took longer than {} s", within.as_secs()),
        }
    }
}

/// The process group of a shell [`take_in`] runs: what the caller kills when it ends first.
/// Clones name the same shell.
#[derive(Debug, Clone, Default)]
pub struct ShellGroup(Arc<AtomicI32>);

impl ShellGroup {
    const NONE: i32 = 0;
    /// The caller ended: a shell started after this is killed as it starts.
    const ENDED: i32 = -1;

    fn started(&self, group: libc::pid_t) {
        if self
            .0
            .compare_exchange(Self::NONE, group, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            kill_group(group);
        }
    }

    /// Notes that the shell was reaped: its group number may be anybody's from now on.
    fn reaped(&self) {
        let _ = self
            .0
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |held| {
                (held > Self::NONE).then_some(Self::NONE)
            });
    }

    pub fn end(&self) {
        let held = self.0.swap(Self::ENDED, Ordering::SeqCst);
        if held > Self::NONE {
            kill_group(held);
        }
    }
}

/// `base`'s values win, so a project's own setup (nix, direnv, a virtual environment) stays. `PATH`
/// is `base`'s entries, then the shell's entries it lacks. An error leaves the caller its own
/// environment; nobody retries.
pub fn take(
    shell: &LoginShell,
    base: &[(OsString, OsString)],
) -> Result<Vec<(OsString, OsString)>, ShellError> {
    take_in(shell, base, &ShellGroup::default())
}

pub fn take_in(
    shell: &LoginShell,
    base: &[(OsString, OsString)],
    group: &ShellGroup,
) -> Result<Vec<(OsString, OsString)>, ShellError> {
    let started = Instant::now();
    let home = base
        .iter()
        .find(|(name, _)| name == "HOME")
        .map(|(_, value)| PathBuf::from(value))
        .filter(|home| home.is_dir())
        .unwrap_or_else(|| PathBuf::from("/"));
    let mut command = Command::new(&shell.program);
    command
        .args(["-l", "-i", "-c", COMMAND])
        .env_clear()
        .envs(base.iter().map(|(name, value)| (name, value)))
        .current_dir(home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(target_os = "linux")]
    // SAFETY: `getpid` has no preconditions.
    let parent = unsafe { libc::getpid() };
    // SAFETY: `setsid`, `prctl`, `getppid` and `_exit` are async-signal-safe and touch no
    // memory of the parent, as a function run between `fork` and `exec` must not.
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            // On Linux the kernel kills the shell when the thread that started it ends, so a
            // program that dies leaves no shell behind; that thread returns only once the shell is
            // reaped.
            #[cfg(target_os = "linux")]
            {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                // The parent may have ended before the request took effect: its signal would never
                // come.
                if libc::getppid() != parent {
                    libc::_exit(1);
                }
            }
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .map_err(|error| ShellError::Start(error.to_string()))?;
    // The session's leader: its id is the id of the whole group.
    let id = child.id() as libc::pid_t;
    group.started(id);
    let mut stdout = child.stdout.take().expect("the output was piped");

    let (output_tx, output) = mpsc::channel();
    std::thread::Builder::new()
        .name("login-shell-output".to_owned())
        .spawn(move || {
            let mut bytes = Vec::new();
            let _ = stdout.read_to_end(&mut bytes);
            let _ = output_tx.send(bytes);
        })
        .map_err(|error| {
            kill_group(id);
            let _ = child.wait();
            ShellError::Start(error.to_string())
        })?;
    let (status_tx, status) = mpsc::channel();
    std::thread::Builder::new()
        .name("login-shell-wait".to_owned())
        .spawn(move || {
            let _ = status_tx.send(child.wait());
        })
        .map_err(|error| {
            kill_group(id);
            ShellError::Start(error.to_string())
        })?;

    let left = |now: Instant| shell.deadline.saturating_sub(now.duration_since(started));
    let ended = match status.recv_timeout(left(Instant::now())) {
        Ok(ended) => ended,
        Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
            // Past the deadline the whole group goes; the reading thread ends once every holder of
            // the pipe is gone.
            kill_group(id);
            let _ = status.recv();
            group.reaped();
            return Err(ShellError::Deadline(shell.deadline));
        }
    };
    // Whatever the shell left in the background must not hold the pipe open.
    group.reaped();
    kill_group(id);
    let ended = ended.map_err(|error| ShellError::Start(error.to_string()))?;
    if !ended.success() {
        return Err(ShellError::Status(ended.code()));
    }
    // A process that left the group (a session of its own) may still hold the pipe: the deadline
    // bounds the wait for the rest of the output.
    let bytes = match output.recv_timeout(left(Instant::now())) {
        Ok(bytes) => bytes,
        Err(_) => return Err(ShellError::Deadline(shell.deadline)),
    };
    let printed = printed_after_marker(&bytes).ok_or(ShellError::NoMarker)?;
    Ok(merge(base, &printed))
}

fn kill_group(group: libc::pid_t) {
    // SAFETY: a signal to the group of a process this function's caller started; a group
    // that no longer exists answers `ESRCH`, which is no fault.
    let _ = unsafe { libc::kill(-group, libc::SIGKILL) };
}

fn printed_after_marker(output: &[u8]) -> Option<Vec<(OsString, OsString)>> {
    let marker = MARKER.as_bytes();
    let at = output
        .windows(marker.len())
        .enumerate()
        .rev()
        .find(|(_, window)| *window == marker)
        .map(|(at, _)| at)?;
    let after = &output[at + marker.len()..];
    Some(
        after
            .split(|byte| *byte == 0)
            .filter_map(|entry| {
                let equals = entry.iter().position(|byte| *byte == b'=')?;
                (equals > 0).then(|| {
                    (
                        OsString::from_vec(entry[..equals].to_vec()),
                        OsString::from_vec(entry[equals + 1..].to_vec()),
                    )
                })
            })
            .collect(),
    )
}

fn merge(
    base: &[(OsString, OsString)],
    printed: &[(OsString, OsString)],
) -> Vec<(OsString, OsString)> {
    let mut merged: Vec<(OsString, OsString)> = base.to_vec();
    for (name, value) in printed {
        if NOT_TAKEN.iter().any(|not| name == OsStr::new(not)) {
            continue;
        }
        match merged.iter().position(|(have, _)| have == name) {
            None => merged.push((name.clone(), value.clone())),
            Some(at) if name == "PATH" => {
                let mut entries: Vec<PathBuf> = std::env::split_paths(&merged[at].1).collect();
                for entry in std::env::split_paths(value) {
                    if !entries.contains(&entry) {
                        entries.push(entry);
                    }
                }
                if let Ok(joined) = std::env::join_paths(entries) {
                    merged[at].1 = joined;
                }
            }
            Some(_) => {}
        }
    }
    merged
}

pub fn search_path(environment: &[(OsString, OsString)]) -> Vec<PathBuf> {
    environment
        .iter()
        .find(|(name, _)| name == "PATH")
        .map(|(_, value)| std::env::split_paths(value).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variables(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).into(), (*value).into()))
            .collect()
    }

    #[test]
    fn the_merge_keeps_the_values_of_the_base_and_adds_what_only_the_shell_has() {
        let merged = merge(
            &variables(&[("A", "base"), ("HOME", "/h")]),
            &variables(&[("A", "shell"), ("B", "only"), ("HOME", "/elsewhere")]),
        );
        assert_eq!(
            merged,
            variables(&[("A", "base"), ("HOME", "/h"), ("B", "only")])
        );
    }

    #[test]
    fn the_merge_never_takes_the_variables_that_describe_the_shells_own_run() {
        let merged = merge(
            &variables(&[("A", "1")]),
            &variables(&[
                ("PWD", "/p"),
                ("OLDPWD", "/o"),
                ("SHLVL", "3"),
                ("_", "/usr/bin/env"),
                ("B", "2"),
            ]),
        );
        assert_eq!(merged, variables(&[("A", "1"), ("B", "2")]));
        let merged = merge(
            &variables(&[("PWD", "/mine")]),
            &variables(&[("PWD", "/p")]),
        );
        assert_eq!(merged, variables(&[("PWD", "/mine")]));
    }

    #[test]
    fn the_merge_gives_the_path_of_the_base_then_the_entries_of_the_shell_it_lacks() {
        let merged = merge(
            &variables(&[("PATH", "/a:/b")]),
            &variables(&[("PATH", "/c:/a:/d")]),
        );
        assert_eq!(merged, variables(&[("PATH", "/a:/b:/c:/d")]));
    }

    #[test]
    fn the_merge_adds_a_path_the_base_lacks_and_keeps_one_the_shell_lacks() {
        assert_eq!(
            merge(&variables(&[("A", "1")]), &variables(&[("PATH", "/c:/d")])),
            variables(&[("A", "1"), ("PATH", "/c:/d")])
        );
        assert_eq!(
            merge(&variables(&[("PATH", "/a")]), &variables(&[("B", "2")])),
            variables(&[("PATH", "/a"), ("B", "2")])
        );
    }

    #[test]
    fn the_variables_are_what_follows_the_last_marker_and_an_entry_with_no_name_is_dropped() {
        let mut output = b"noise\n".to_vec();
        output.extend_from_slice(MARKER.as_bytes());
        output.extend_from_slice(b"stale=1\0");
        output.extend_from_slice(MARKER.as_bytes());
        output.extend_from_slice(b"A=1\0NOEQUALS\0=x\0B=two=parts\0C=\0multi=line\nvalue\0");
        let printed = printed_after_marker(&output).expect("a marker");
        assert_eq!(
            printed,
            variables(&[
                ("A", "1"),
                ("B", "two=parts"),
                ("C", ""),
                ("multi", "line\nvalue"),
            ])
        );
        assert_eq!(printed_after_marker(b"A=1\0"), None);
    }

    #[test]
    fn the_search_path_is_the_path_split_and_none_where_there_is_no_path() {
        assert_eq!(
            search_path(&variables(&[("X", "1"), ("PATH", "/a:/b")])),
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
        assert_eq!(
            search_path(&variables(&[("X", "1")])),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn each_failure_is_one_sentence_a_program_writes_after_its_own_name() {
        assert_eq!(
            ShellError::Start("no such file".to_owned()).to_string(),
            "it could not be started: no such file"
        );
        assert_eq!(
            ShellError::Status(Some(3)).to_string(),
            "it ended with status 3"
        );
        assert_eq!(
            ShellError::Status(None).to_string(),
            "it was ended by a signal"
        );
        assert_eq!(
            ShellError::NoMarker.to_string(),
            "it printed no environment"
        );
        assert_eq!(
            ShellError::Deadline(Duration::from_secs(10)).to_string(),
            "it took longer than 10 s"
        );
    }
}

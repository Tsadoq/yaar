//! The environment of the user's login shell, taken once: what a terminal of the app has, for
//! a program the system started with its own short search path.
//!
//! A program started from the Finder or the Dock on macOS, or from a desktop's menu on Linux,
//! has the system's environment: on macOS a search path of `/usr/bin:/bin:/usr/sbin:/sbin` and
//! few variables. The Dock's terminals start the user's login shell and so have what the
//! user's start-up files set; a program of the app that runs the user's own programs without a
//! terminal takes the same by running that shell once ([`take`]) and reading the environment
//! it prints. Two programs do: the Window, for its language servers (the Navigator, which says
//! when it takes it), and the Keeper, for the user's git it runs and the hooks that git runs.
//!
//! # How
//!
//! [`LoginShell::program`] is run with the arguments `-l`, `-i`, `-c` and [`COMMAND`], as the
//! Dock's launchers run it: a login shell that is interactive, so that it reads the start-up
//! files a terminal of the app reads. It runs
//! - in the environment `base` the caller hands, and the folder `HOME` names there, or `/`
//!   when it names none or names no folder;
//! - in a session of its own (`setsid` before `exec`), with its standard input from
//!   `/dev/null` and its standard error thrown away, so that it can neither take a terminal
//!   nor wait for a key;
//! - for at most [`LoginShell::deadline`] of real time, counted from its start: past it, its
//!   whole process group is killed (`kill(-pid, SIGKILL)`) and reaped, and the take has
//!   failed with [`ShellError::Deadline`];
//! - never for longer than its caller wants: [`ShellGroup::end`] kills the group of a shell
//!   still running, and on Linux the kernel kills the shell when the thread that started it
//!   ends (`PR_SET_PDEATHSIG`, with the parent checked with `getppid` after the request, so
//!   that a parent already gone ends the child at once).
//!
//! Two threads read what the shell leaves: one reads its standard output to the end, and one
//! waits for the shell and reaps it, while the caller waits on that thread's channel for what
//! is left of the deadline. Once the shell has exited, its whole process group is killed at
//! once (an error of "no such process" being none): a job a start-up file left in the
//! background would otherwise hold the pipe open, and the read would wait for it. The rest of
//! the output is then awaited only within what is left of the deadline; past it the take fails
//! with the deadline.
//!
//! [`COMMAND`] prints [`MARKER`] and then has `env -0` print every variable, each `NAME=value`
//! and a NUL. Whatever a start-up file printed before is noise and is dropped: the variables
//! are what follows the last [`MARKER`] of the standard output. Each is split at its first `=`;
//! one with no `=` or an empty name is dropped.
//!
//! # What the caller gets
//!
//! `base` first: a variable `base` has keeps `base`'s value, so that a program started from a
//! shell of a project's own setup (a `nix develop`, a `direnv` folder, a Python virtual
//! environment) keeps that setup. Every variable the shell printed that `base` lacks is added;
//! but `PWD`, `OLDPWD`, `SHLVL` and `_`, which describe the shell's run and not the user's
//! setup, are never taken from the shell. `PATH` is the one merged: `base`'s entries, in their
//! order, then each entry of the shell's `PATH` that `base`'s does not hold, in the shell's
//! order. A program started from the Finder or the Dock has the system's short path and few
//! variables, and so gains the folders and the variables of the user's start-up files.
//!
//! # When it fails
//!
//! A shell that cannot be started, ends with a status other than 0, prints no [`MARKER`], or
//! outlives its deadline gives a [`ShellError`]; the caller keeps its own environment and says
//! why in one line of its standard error. Nobody tries again.
//!
//! A test hands a shell of its own making (a script that records its arguments, prints what
//! the test wants and then runs the command it was given with `/bin/sh -c`), never the user's,
//! and a `HOME` in its scratch folder.
//!
//! This module is the Navigator's `shell.rs` of slice 9 moved here, so that the Keeper takes
//! the same environment by the same rules; the Navigator keeps its names by re-exporting
//! these.

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

/// How long the user's login shell may take to print its environment: ten seconds, a slow
/// start-up file's time and more.
pub const SHELL_DEADLINE: Duration = Duration::from_secs(10);

/// What the shell prints before the variables, so that what its start-up files printed
/// before can be told from them.
pub const MARKER: &str = "yaar-environment-follows";

/// The command the shell is told to run.
pub const COMMAND: &str = "printf '%s' 'yaar-environment-follows'; exec env -0";

/// The variables that describe the shell's own run, not the user's setup: never taken from
/// the shell.
const NOT_TAKEN: [&str; 4] = ["PWD", "OLDPWD", "SHLVL", "_"];

/// The user's login shell, run once for its environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginShell {
    /// The shell: [`crate::user_shell`] for the programs, a script of its own for a test.
    pub program: PathBuf,
    /// How long it may take: [`SHELL_DEADLINE`] for the programs.
    pub deadline: Duration,
}

/// Why the shell gave no environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellError {
    /// It could not be started.
    Start(String),
    /// It ended with this status, or by a signal (none).
    Status(Option<i32>),
    /// Its output held no [`MARKER`].
    NoMarker,
    /// It ran past its deadline and was killed.
    Deadline(Duration),
}

impl fmt::Display for ShellError {
    /// One sentence a program writes after its own name: "it could not be started: …", "it
    /// ended with status 3", "it was ended by a signal", "it printed no environment", "it
    /// took longer than 10 s".
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

/// The process group of a shell [`take_in`] runs, while it runs: what its caller kills when
/// it ends first. Cheap to clone; every clone names the same shell.
#[derive(Debug, Clone, Default)]
pub struct ShellGroup(Arc<AtomicI32>);

impl ShellGroup {
    /// No shell yet.
    const NONE: i32 = 0;
    /// The caller ended: a shell started after this is killed as it starts.
    const ENDED: i32 = -1;

    /// Notes that the shell's group is `group`; kills it at once when the caller has
    /// ended already.
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

    /// Kills the whole process group of the shell that runs now, and makes a shell that
    /// [`take_in`] starts after this call be killed at its start. Does nothing more when no
    /// shell runs.
    pub fn end(&self) {
        let held = self.0.swap(Self::ENDED, Ordering::SeqCst);
        if held > Self::NONE {
            kill_group(held);
        }
    }
}

/// Runs `shell` once, as the module doc says, in the environment `base`, and gives `base`
/// with what the shell printed merged in, as the module doc says.
pub fn take(
    shell: &LoginShell,
    base: &[(OsString, OsString)],
) -> Result<Vec<(OsString, OsString)>, ShellError> {
    take_in(shell, base, &ShellGroup::default())
}

/// [`take`], noting the shell's process group in `group` while it runs.
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
            // On Linux the kernel kills the shell when the thread that started it ends, so
            // that a program that dies leaves no shell behind. The thread outlives the
            // shell: it returns once the shell is reaped.
            #[cfg(target_os = "linux")]
            {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                // The parent may have ended before the request took effect: its signal
                // would never come.
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
            // Past its deadline: the whole group goes, and the wait returns once it is
            // reaped. The reading thread ends by itself when every holder of the pipe is
            // gone.
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
    // A process that left the group (a session of its own) may still hold the pipe: the
    // deadline bounds the wait for the rest of the output.
    let bytes = match output.recv_timeout(left(Instant::now())) {
        Ok(bytes) => bytes,
        Err(_) => return Err(ShellError::Deadline(shell.deadline)),
    };
    let printed = printed_after_marker(&bytes).ok_or(ShellError::NoMarker)?;
    Ok(merge(base, &printed))
}

/// Kills every process of the process group `group`. A group that is gone is no error.
fn kill_group(group: libc::pid_t) {
    // SAFETY: a signal to the group of a process this function's caller started; a group
    // that no longer exists answers `ESRCH`, which is no fault.
    let _ = unsafe { libc::kill(-group, libc::SIGKILL) };
}

/// The variables the shell printed: what follows the last [`MARKER`] of `output`, each
/// `NAME=value` ended by a NUL, split at the first `=`. An entry with no `=` or with an
/// empty name is dropped. None when `output` holds no marker.
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

/// `base` with the variables of `printed` it lacks added, but the ones of [`NOT_TAKEN`],
/// and its `PATH` followed by the entries of the printed one it does not hold.
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

/// The search path of an environment: its `PATH`, split as `std::env::split_paths` splits it;
/// none where it has no `PATH`.
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
        // The base's own value of one of them stays.
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
        // A start-up file that printed the marker too: only what follows the last counts.
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

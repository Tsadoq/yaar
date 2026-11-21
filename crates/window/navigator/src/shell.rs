//! The environment of the language servers, taken once from the user's login shell.
//!
//! A Window started from a terminal has the terminal's environment, and its servers run in
//! it. A Window started from the Finder or the Dock on macOS, or from a desktop's menu on
//! Linux, has the system's short search path: no server of rustup, Homebrew or nvm is found,
//! and a downloaded rust-analyzer finds no `cargo`. The Dock's terminals start the user's
//! login shell and so have what the user's start-up files set; the servers take the same,
//! by running that shell once ([`take`]) and reading the environment it prints.
//!
//! # When
//!
//! Never before a server is needed: the Navigator runs the shell when it first has to
//! search for a row's candidates (`lifecycle.rs`), not at its start, and not for a Window
//! that never shows a workspace with a server's file. Once per Navigator: the answer, or the
//! failure, holds for its life. While the shell runs, the keys that wait for a search are
//! held (`lifecycle.rs`, "Where a key stands"): their state is `Starting` with no process,
//! and asks about their files are answered by text match, as for any server not ready.
//! The shell runs on a thread of its own and reports with `Event::Shell`; the Navigator's
//! thread never waits for it.
//!
//! # How
//!
//! [`LoginShell::program`] is run with the arguments `-l`, `-i`, `-c` and [`COMMAND`], as the
//! Dock's launchers run it: a login shell that is interactive, so that it reads the start-up
//! files a terminal of the app reads. It runs
//! - in the Window's own environment (`NavigatorConfig::environment`) and the folder `HOME`
//!   names there, or `/` when it names none;
//! - in a session of its own (`setsid` before `exec`), with its standard input from
//!   `/dev/null` and its standard error thrown away, so that it can neither take a terminal
//!   nor wait for a key;
//! - for at most [`LoginShell::deadline`] of real time: past it, its whole process group is
//!   killed (`kill(-pid, SIGKILL)`) and reaped, and the take has failed.
//!
//! Its standard output is read to its end on a second thread while the first waits for the
//! shell with a polling `try_wait` up to the deadline. Once the shell has exited, its whole
//! process group is killed at once (`kill(-pid, SIGKILL)`, an error of "no such process"
//! being none): a job a start-up file left in the background would otherwise hold the pipe
//! open, and the read would wait for it until the deadline. The output is then what the
//! reading thread has once the pipe is closed.
//!
//! [`COMMAND`] prints [`MARKER`] and then has `env -0` print every variable, each
//! `NAME=value` and a NUL. Whatever a start-up file printed before is noise and is dropped:
//! the variables are what follows the last [`MARKER`] of the standard output. Each is split
//! at its first `=`; one with no `=` or an empty name is dropped.
//!
//! # What the servers get
//!
//! The Window's own environment first: a variable the Window has keeps the Window's value,
//! so that a Window started from a shell of a project's own setup (a `nix develop`, a
//! `direnv` folder, a Python virtual environment) gives its servers that setup, as before
//! slice 9. Every variable the shell printed that the Window lacks is added; but `PWD`,
//! `OLDPWD`, `SHLVL` and `_`, which describe the shell's run and not the user's setup, are
//! never taken from the shell. `PATH` is the one merged: the Window's entries, in their
//! order, then each entry of the shell's `PATH` that the Window's does not hold, in the
//! shell's order. The search path the candidates are looked for in
//! (`NavigatorConfig::search_path`) is the `PATH` of the result, split as
//! `std::env::split_paths` splits it. A Window started from the Finder or the Dock has the
//! system's short path and few variables, and so gains the folders and the variables of the
//! user's start-up files.
//!
//! # When it fails
//!
//! A shell that cannot be started, ends with a status other than 0, prints no [`MARKER`], or
//! outlives its deadline leaves the servers the Window's own environment and search path, as
//! before slice 9, and the Navigator writes one line to its standard error, starting
//! `yaar-navigator:`, with the shell's path and why. It is not tried again.
//!
//! A test hands a shell of its own making (a script that records its arguments, prints what
//! the test wants and then runs the command it was given with `/bin/sh -c`), never the
//! user's, and a `HOME` in its scratch folder.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::Read;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
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

/// The user's login shell, run once for the environment of the language servers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginShell {
    /// The shell: `yaar_link::user_shell()` for the Window, a script of its own for a test.
    pub program: PathBuf,
    /// How long it may take: [`SHELL_DEADLINE`] for the Window.
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start(why) => write!(f, "it could not be started: {why}"),
            Self::Status(Some(code)) => write!(f, "it ended with status {code}"),
            Self::Status(None) => write!(f, "a signal ended it"),
            Self::NoMarker => write!(f, "it printed no environment"),
            Self::Deadline(within) => write!(f, "it took longer than {} s", within.as_secs()),
        }
    }
}

/// Runs `shell` once, as the module doc says, in the environment `base`, and gives the
/// environment the servers run in: `base` with what the shell printed merged in, as the
/// module doc says.
pub fn take(
    shell: &LoginShell,
    base: &[(OsString, OsString)],
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
    // SAFETY: `setsid` is async-signal-safe and touches no memory of the parent, as a
    // function run between `fork` and `exec` must not.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .map_err(|error| ShellError::Start(error.to_string()))?;
    // The session's leader: its id is the id of the whole group.
    let group = child.id() as libc::pid_t;
    let mut stdout = child.stdout.take().expect("the output was piped");

    let (output_tx, output) = mpsc::channel();
    std::thread::Builder::new()
        .name("navigator-shell-output".to_owned())
        .spawn(move || {
            let mut bytes = Vec::new();
            let _ = stdout.read_to_end(&mut bytes);
            let _ = output_tx.send(bytes);
        })
        .map_err(|error| {
            kill_group(group);
            let _ = child.wait();
            ShellError::Start(error.to_string())
        })?;
    let (status_tx, status) = mpsc::channel();
    std::thread::Builder::new()
        .name("navigator-shell-wait".to_owned())
        .spawn(move || {
            let _ = status_tx.send(child.wait());
        })
        .map_err(|error| {
            kill_group(group);
            ShellError::Start(error.to_string())
        })?;

    let left = |now: Instant| shell.deadline.saturating_sub(now.duration_since(started));
    let ended = match status.recv_timeout(left(Instant::now())) {
        Ok(ended) => ended,
        Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
            // Past its deadline: the whole group goes, and the wait returns once it is
            // reaped. The reading thread ends by itself when every holder of the pipe is
            // gone.
            kill_group(group);
            let _ = status.recv();
            return Err(ShellError::Deadline(shell.deadline));
        }
    };
    // Whatever the shell left in the background must not hold the pipe open.
    kill_group(group);
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

/// The search path of an environment: its `PATH`, split; none where it has no `PATH`.
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
        let merged = merge(&variables(&[("PWD", "/mine")]), &variables(&[("PWD", "/p")]));
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
        assert_eq!(search_path(&variables(&[("X", "1")])), Vec::<PathBuf>::new());
    }
}

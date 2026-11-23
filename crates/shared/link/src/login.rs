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

use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

/// How long the user's login shell may take to print its environment: ten seconds, a slow
/// start-up file's time and more.
pub const SHELL_DEADLINE: Duration = Duration::from_secs(10);

/// What the shell prints before the variables, so that what its start-up files printed
/// before can be told from them.
pub const MARKER: &str = "yaar-environment-follows";

/// The command the shell is told to run.
pub const COMMAND: &str = "printf '%s' 'yaar-environment-follows'; exec env -0";

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
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!()
    }
}

/// The process group of a shell [`take_in`] runs, while it runs: what its caller kills when
/// it ends first. Cheap to clone; every clone names the same shell.
#[derive(Debug, Clone, Default)]
pub struct ShellGroup {
    // Filled by the builder: the group's id while the shell runs, none before and after.
    _group: (),
}

impl ShellGroup {
    /// Kills the whole process group of the shell that runs now, and makes a shell that
    /// [`take_in`] starts after this call be killed at its start. Does nothing more when no
    /// shell runs.
    pub fn end(&self) {
        todo!()
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
    _shell: &LoginShell,
    _base: &[(OsString, OsString)],
    _group: &ShellGroup,
) -> Result<Vec<(OsString, OsString)>, ShellError> {
    todo!()
}

/// The search path of an environment: its `PATH`, split as `std::env::split_paths` splits it;
/// none where it has no `PATH`.
pub fn search_path(_environment: &[(OsString, OsString)]) -> Vec<PathBuf> {
    todo!()
}

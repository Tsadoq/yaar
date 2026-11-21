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
    let _ = (shell, base);
    todo!("run the shell, read past the marker, merge, as the module doc says")
}

/// The search path of an environment: its `PATH`, split; none where it has no `PATH`.
pub fn search_path(environment: &[(OsString, OsString)]) -> Vec<PathBuf> {
    let _ = environment;
    todo!("split the PATH of the environment")
}

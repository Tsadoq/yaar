//! The one place that starts git.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The variables with which a process tells git which repository to use, whatever
/// folder it runs in. A Keeper started from a shell that has one of them set would read
/// that repository in place of the one it was asked about, so no run of git here
/// inherits them.
const REPOSITORY_VARIABLES: [&str; 8] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_PREFIX",
];

/// The git program of the user, as the app runs it to read.
///
/// Cheap to clone: it holds only where the program is.
#[derive(Debug, Clone)]
pub struct Git {
    program: PathBuf,
}

/// Where a run of git looks for its repository.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Place<'a> {
    /// In this folder, or above it, as git finds it.
    Folder(&'a Path),
    /// In this git folder, named outright.
    GitDir(&'a Path),
}

/// What one run of git gave.
#[derive(Debug)]
pub(crate) struct Answer {
    /// The command, as one would type it.
    command: String,
    /// Its exit code; none when a signal ended it.
    pub(crate) code: Option<i32>,
    /// What it printed.
    pub(crate) stdout: Vec<u8>,
    /// What it wrote to standard error.
    pub(crate) stderr: String,
}

impl Answer {
    /// What git printed, when it ended well.
    pub(crate) fn printed(self) -> Result<Vec<u8>, GitError> {
        if self.code == Some(0) {
            Ok(self.stdout)
        } else {
            Err(self.failed())
        }
    }

    /// The run as a failure of git.
    pub(crate) fn failed(self) -> GitError {
        GitError::Failed {
            command: self.command,
            code: self.code,
            stderr: self.stderr,
        }
    }

    /// The run as an answer that was not understood.
    pub(crate) fn unreadable(&self, what: impl Into<String>) -> GitError {
        unreadable(&self.command, what)
    }

    /// The command, as one would type it.
    pub(crate) fn command(&self) -> &str {
        &self.command
    }
}

/// An answer of `command` that was not understood.
pub(crate) fn unreadable(command: &str, what: impl Into<String>) -> GitError {
    GitError::Unreadable {
        command: command.to_owned(),
        what: what.into(),
    }
}

impl Git {
    /// The `git` on the search path of this process.
    pub fn find() -> Result<Git, GitError> {
        let search = std::env::var_os("PATH").ok_or(GitError::NotFound)?;
        std::env::split_paths(&search)
            // An entry that is not a full path means a folder under wherever this
            // process happens to run: not a place to take a program from.
            .filter(|folder| folder.is_absolute())
            .map(|folder| folder.join("git"))
            .find(|candidate| is_program(candidate))
            .map(|program| Git { program })
            .ok_or(GitError::NotFound)
    }

    /// The git program at this path.
    pub fn at(program: impl Into<PathBuf>) -> Git {
        Git {
            program: program.into(),
        }
    }

    /// Where the program is.
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// A run of git that reads, not yet started, with everything every read carries:
    /// no hook of the hooks folder, no file-system monitor, each hook the configuration
    /// defines turned off by its key in `hooks_off`, no lock that can be done without,
    /// no question at a terminal, and git's own language.
    pub(crate) fn command(&self, place: Place<'_>, hooks_off: &[OsString]) -> Command {
        let mut command = Command::new(&self.program);
        command.args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
        ]);
        match place {
            Place::Folder(folder) => {
                command.arg("-C").arg(folder);
            }
            Place::GitDir(git_dir) => {
                let mut option = OsString::from("--git-dir=");
                option.push(git_dir);
                command.arg(option);
            }
        }
        for name in REPOSITORY_VARIABLES {
            command.env_remove(name);
        }
        command
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .stdin(Stdio::null());

        // The name of a hook can hold any character, an equals sign too, which `-c`
        // would cut at. Settings handed over in the environment are taken whole. They
        // are numbered after those the environment of this process already holds.
        if !hooks_off.is_empty() {
            let before = std::env::var("GIT_CONFIG_COUNT")
                .ok()
                .and_then(|count| count.parse::<usize>().ok())
                .unwrap_or(0);
            for (n, key) in hooks_off.iter().enumerate() {
                command
                    .env(format!("GIT_CONFIG_KEY_{}", before + n), key)
                    .env(format!("GIT_CONFIG_VALUE_{}", before + n), "false");
            }
            command.env("GIT_CONFIG_COUNT", (before + hooks_off.len()).to_string());
        }
        command
    }

    /// A run of the user's own git, not yet started, as the user would start it in
    /// `folder`: none of the settings every read carries, so that the user's hooks,
    /// identity, signing and language apply. Only the variables that name another
    /// repository are left out, and nothing is asked at a terminal.
    pub(crate) fn plain(&self, folder: &Path) -> Command {
        let mut command = Command::new(&self.program);
        command.arg("-C").arg(folder);
        for name in REPOSITORY_VARIABLES {
            command.env_remove(name);
        }
        command.env("GIT_TERMINAL_PROMPT", "0").stdin(Stdio::null());
        command
    }

    /// Runs git to its end and gives what it answered, whatever its exit code.
    pub(crate) fn run<I, S>(
        &self,
        place: Place<'_>,
        hooks_off: &[OsString],
        args: I,
    ) -> Result<Answer, GitError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = self.command(place, hooks_off);
        command.args(args);
        finish(command)
    }
}

/// Runs a prepared command to its end.
pub(crate) fn finish(mut command: Command) -> Result<Answer, GitError> {
    let shown = shown(&command);
    let output = command.output().map_err(could_not_start)?;
    Ok(Answer {
        command: shown,
        code: output.status.code(),
        stdout: output.stdout,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Runs a prepared command to its end with `input` on its standard input.
pub(crate) fn finish_with(mut command: Command, input: &[u8]) -> Result<Answer, GitError> {
    let shown = shown(&command);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(could_not_start)?;
    let Some(mut asks) = child.stdin.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::other("git was started without its input").into());
    };
    // Written on a thread of its own, so that git can fill its output while it reads:
    // a pipe that nobody empties would stop both. A write that fails means git ended
    // before it read all, and its exit code says why.
    let output = std::thread::scope(|scope| {
        scope.spawn(move || {
            let _ = asks.write_all(input);
        });
        child.wait_with_output()
    })?;
    Ok(Answer {
        command: shown,
        code: output.status.code(),
        stdout: output.stdout,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Why git did not start.
pub(crate) fn could_not_start(e: io::Error) -> GitError {
    if e.kind() == io::ErrorKind::NotFound {
        GitError::NotFound
    } else {
        GitError::Io(e)
    }
}

/// The command as one would type it: `git` and what was asked, without the settings
/// every run carries.
pub(crate) fn shown(command: &Command) -> String {
    let mut shown = String::from("git");
    let mut args = command.get_args();
    while let Some(arg) = args.next() {
        if arg == "-c" || arg == "-C" {
            args.next();
        } else {
            shown.push(' ');
            shown.push_str(&arg.to_string_lossy());
        }
    }
    shown
}

fn is_program(path: &Path) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|file| file.is_file() && file.permissions().mode() & 0o111 != 0)
}

/// Why git gave no answer.
#[derive(Debug)]
pub enum GitError {
    /// There is no git program to run.
    NotFound,
    /// git could not be started, or its output could not be read.
    Io(io::Error),
    /// git ran and failed.
    Failed {
        /// The command, as one would type it.
        command: String,
        /// Its exit code; none when a signal ended it.
        code: Option<i32>,
        /// What it wrote to standard error.
        stderr: String,
    },
    /// git answered with something this crate does not understand.
    Unreadable {
        /// The command, as one would type it.
        command: String,
        /// What was not understood.
        what: String,
    },
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => write!(f, "git was not found"),
            Self::Io(e) => write!(f, "git could not be run: {e}"),
            Self::Failed {
                command,
                code,
                stderr,
            } => match code {
                Some(code) => write!(f, "`{command}` failed with {code}: {}", stderr.trim()),
                None => write!(f, "`{command}` was stopped: {}", stderr.trim()),
            },
            Self::Unreadable { command, what } => {
                write!(f, "`{command}` answered what is not understood: {what}")
            }
        }
    }
}

impl std::error::Error for GitError {}

impl From<io::Error> for GitError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

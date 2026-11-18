//! The one place that starts git.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// The git program of the user, as the app runs it to read.
///
/// Cheap to clone: it holds only where the program is.
#[derive(Debug, Clone)]
pub struct Git {
    program: PathBuf,
}

impl Git {
    /// The `git` on the search path of this process.
    pub fn find() -> Result<Git, GitError> {
        todo!()
    }

    /// The git program at this path.
    pub fn at(program: impl Into<PathBuf>) -> Git {
        todo!()
    }

    /// Where the program is.
    pub fn program(&self) -> &Path {
        todo!()
    }
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

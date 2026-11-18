//! Store (`H-STORE`).
//!
//! Owns: the SQLite database and its one writer: reviews, sessions, steps of turns, marks, settings.
//!
//! Surface: [`Store`], opened on one database file, and what it keeps so far: the
//! repositories the user opened ([`RepositoryRecord`]) and their workspaces
//! ([`WorkspaceRecord`]), each with the base the user set for it, under the ids
//! [`RepositoryId`] and [`WorkspaceId`].
//!
//! One database file holds everything, for all workspaces, in the app's folder; nothing of
//! it lies in a worktree or in a repository. No other crate opens the file or knows its
//! tables: the parts of the Keeper keep and find their records through the calls here.
//!
//! One thread writes. Each call below that changes something is one transaction, handed
//! to that thread, and returns when the change is on the drive: what such a call has
//! returned is still there after the Keeper is killed. A call that saves without waiting
//! for the drive says so in its own words.
//!
//! The tables carry a version. A database of an older version is brought forward change
//! by change when it is opened, after the file was copied beside itself; one of a newer
//! version is refused, and left as it is.
//!
//! What goes wrong is told in words of this crate: no caller sees the database library.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// The name of the database file in the app's folder.
pub const FILE_NAME: &str = "store.db";

/// Names one workspace: 32 lowercase hexadecimal digits, drawn at random when its
/// worktree is first seen, and written into the git folder of that worktree.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkspaceId(String);

impl WorkspaceId {
    /// The id with this text.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The text of the id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Names one repository: 32 lowercase hexadecimal digits, drawn at random when the
/// repository is first opened and kept here with the git folder of the repository.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepositoryId(String);

impl RepositoryId {
    /// The id with this text.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The text of the id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RepositoryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A repository the user opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryRecord {
    /// Its id, as Workspaces made it.
    pub id: RepositoryId,
    /// The git folder its worktrees share, where it was last seen.
    pub git_dir: PathBuf,
}

/// A workspace: one worktree of an opened repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRecord {
    /// Its id, as it stands in the git folder of the worktree.
    pub id: WorkspaceId,
    /// The id of its repository.
    pub repository: RepositoryId,
    /// The folder of the worktree, where it was last seen.
    pub path: PathBuf,
    /// The base the user set, in the words Workspaces keeps it in; none while the
    /// default stands.
    pub base: Option<String>,
}

/// Why the Store could not do what was asked.
#[derive(Debug)]
pub enum StoreError {
    /// The database file or its folder could not be made or opened.
    Open {
        /// The file that was to be opened.
        path: PathBuf,
        /// What went wrong, as a sentence.
        reason: String,
    },
    /// The database was written by a later version of the app.
    Newer {
        /// The version of the tables in the file.
        found: u32,
        /// The newest version this build knows.
        known: u32,
    },
    /// The database refused a read or a write; the sentence says what it said.
    Database(String),
    /// The writer is gone: the Store was shut, or its thread died.
    Closed,
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open { path, reason } => {
                write!(f, "could not open {}: {reason}", path.display())
            }
            Self::Newer { found, known } => write!(
                f,
                "the database is of version {found}, and this app knows up to {known}"
            ),
            Self::Database(e) => write!(f, "the database failed: {e}"),
            Self::Closed => write!(f, "the database is closed"),
        }
    }
}

impl std::error::Error for StoreError {}

/// What is handed to the writer: one piece of work on the one connection.
type Job = Box<dyn FnOnce(&mut rusqlite::Connection) + Send>;

/// The database, and the way to its one writer.
///
/// Cheap to clone, and every clone reaches the same writer; it may be used from several
/// threads at once. The writer stops, and the file is closed, when the last clone is
/// dropped.
#[derive(Debug, Clone)]
pub struct Store {
    writer: mpsc::Sender<Job>,
}

impl Store {
    /// Opens the database at `file`, making the file and its folder when they are not
    /// there, and bringing older tables forward. Before the first change to an older
    /// database the file is copied beside itself, under its name and the version it had.
    ///
    /// One process opens a database at a time; that is the Keeper's to see to.
    pub fn open(file: &Path) -> Result<Store, StoreError> {
        todo!()
    }

    /// Every repository the user opened, in the order they were first kept.
    pub fn repositories(&self) -> Result<Vec<RepositoryRecord>, StoreError> {
        todo!()
    }

    /// Keeps a repository: new when its id is not known, else its git folder is brought
    /// up to date.
    pub fn put_repository(&self, record: &RepositoryRecord) -> Result<(), StoreError> {
        todo!()
    }

    /// Every workspace of every repository, in the order they were first kept.
    pub fn workspaces(&self) -> Result<Vec<WorkspaceRecord>, StoreError> {
        todo!()
    }

    /// Keeps a workspace: new when its id is not known, else its repository and its
    /// folder are brought up to date. The base the user set for it is left as it is.
    pub fn put_workspace(
        &self,
        id: &WorkspaceId,
        repository: &RepositoryId,
        path: &Path,
    ) -> Result<(), StoreError> {
        todo!()
    }

    /// Sets the base the user chose for a workspace. A workspace the Store does not hold
    /// is left alone.
    pub fn set_base(&self, workspace: &WorkspaceId, base: &str) -> Result<(), StoreError> {
        todo!()
    }
}

//! Store (`H-STORE`).
//!
//! Owns: the SQLite database and its one writer: reviews, sessions, steps of turns, marks, settings.
//!
//! Surface: [`Store`], opened on one database file, and what it keeps so far: the
//! repositories the user opened ([`RepositoryRecord`]) and their workspaces
//! ([`WorkspaceRecord`]), each with the base the user set for it.
//!
//! One database file holds everything, for all workspaces, in the app's folder; nothing of
//! it lies in a worktree or in a repository. No other crate opens the file or knows its
//! tables: the parts of the Keeper keep and find their records through the calls here.
//!
//! One thread writes. Every call that changes something is one transaction, handed to
//! that thread, and returns when the change is on the drive: what a call has returned is
//! still there after the Keeper is killed.
//!
//! The tables carry a version. A database of an older version is brought forward change
//! by change when it is opened; one of a newer version is refused, and left as it is.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// The name of the database file in the app's folder.
pub const FILE_NAME: &str = "store.db";

/// A repository the user opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryRecord {
    /// Its id, as Workspaces made it.
    pub id: String,
    /// The git folder its worktrees share, where it was last seen.
    pub git_dir: PathBuf,
}

/// A workspace: one worktree of an opened repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRecord {
    /// Its id, as it stands in the git folder of the worktree.
    pub id: String,
    /// The id of its repository.
    pub repository: String,
    /// The folder of the worktree, where it was last seen.
    pub path: PathBuf,
    /// The base the user set, as they wrote it; none while the default stands.
    pub base: Option<String>,
}

/// Why the Store could not do what was asked.
#[derive(Debug)]
pub enum StoreError {
    /// The database file or its folder could not be made or opened.
    Open(PathBuf, rusqlite::Error),
    /// The database was written by a later version of the app.
    Newer {
        /// The version of the tables in the file.
        found: u32,
        /// The newest version this build knows.
        known: u32,
    },
    /// The database refused a read or a write.
    Database(rusqlite::Error),
    /// The writer is gone: the Store was shut, or its thread died.
    Closed,
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open(path, e) => write!(f, "could not open {}: {e}", path.display()),
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
    /// there, and bringing older tables forward.
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
    pub fn put_workspace(&self, id: &str, repository: &str, path: &Path) -> Result<(), StoreError> {
        todo!()
    }

    /// Sets the base the user chose for a workspace, as they wrote it. A workspace the
    /// Store does not hold is left alone.
    pub fn set_base(&self, workspace: &str, base: &str) -> Result<(), StoreError> {
        todo!()
    }
}

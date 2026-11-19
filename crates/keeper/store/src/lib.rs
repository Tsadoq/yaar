//! Store (`H-STORE`).
//!
//! Owns: the SQLite database and its one writer: reviews, sessions, steps of turns, marks, settings.
//!
//! Surface: [`Store`], opened on one database file, and what it keeps:
//! - the repositories the user opened ([`RepositoryRecord`]) and their workspaces
//!   ([`WorkspaceRecord`]), each with the base the user set for it, under the ids
//!   [`RepositoryId`] and [`WorkspaceId`];
//! - the reviews of each workspace ([`ReviewRecord`]) with their [`Thread`]s, each on an
//!   [`Anchor`], their [`Comment`]s and [`Round`]s, under the ids [`ReviewId`],
//!   [`ThreadId`] and [`CommentId`]; and the files marked viewed ([`ViewedRecord`]).
//!
//! One database file holds everything, for all workspaces, in the app's folder; nothing of
//! it lies in a worktree or in a repository. No other crate opens the file or knows its
//! tables: the parts of the Keeper keep and find their records through the calls here.
//!
//! One thread writes. Each call below that changes something is one transaction, handed
//! to that thread, and returns when the change is on the drive: what such a call has
//! returned is still there after the Keeper is killed, and after the machine loses its
//! power. One call saves without waiting for the drive, [`Store::save_draft`], made at
//! every keystroke of a draft: what it has returned is still there after the Keeper is
//! killed, and the writer flushes the drive once the typing pauses. See the module
//! `reviews.rs` for both.
//!
//! The tables carry a version, SQLite's `user_version`. Version 1 holds the repositories
//! and the workspaces; version 2 adds the reviews and the files marked viewed. A database
//! of an older version is brought forward step by step when it is opened, and before the
//! first step the file is copied beside itself, as `store.db.v<its version>`, so that a
//! step that goes wrong loses nothing. A database of a newer version is refused, and left
//! as it is.
//!
//! What goes wrong is told in words of this crate: no caller sees the database library.

use std::ffi::OsString;
use std::fmt;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;

use rusqlite::Connection;

mod reviews;

pub use reviews::{
    Anchor, Author, Comment, CommentId, FLUSH_AFTER, ReviewId, ReviewRecord, Round, SentBy, Side,
    Thread, ThreadId, ViewedRecord,
};

/// The name of the database file in the app's folder.
pub const FILE_NAME: &str = "store.db";

/// The version of the tables this build makes and reads.
const VERSION: u32 = 1;

/// The tables of version 1. `seq` is the order in which the rows were first kept. A
/// path is kept as its bytes, since a path need not be text.
const TABLES: &str = "
    CREATE TABLE repository (
        seq INTEGER PRIMARY KEY,
        id TEXT NOT NULL UNIQUE,
        git_dir BLOB NOT NULL
    ) STRICT;
    CREATE TABLE workspace (
        seq INTEGER PRIMARY KEY,
        id TEXT NOT NULL UNIQUE,
        repository TEXT NOT NULL REFERENCES repository (id),
        path BLOB NOT NULL,
        base TEXT
    ) STRICT;
    PRAGMA user_version = 1;
";

/// What version 2 adds to the tables of version 1: the reviews, their threads, comments
/// and rounds, and the files marked viewed. A path is kept as its bytes; a side, an
/// author and a way of sending are small numbers, in the order their types list them.
/// A comment whose `round` is null is a draft.
#[expect(
    dead_code,
    reason = "the tables of version 2, which the builder of the Store applies"
)]
const TABLES_2: &str = "
    CREATE TABLE review (
        seq INTEGER PRIMARY KEY,
        id TEXT NOT NULL UNIQUE,
        workspace TEXT NOT NULL,
        agent TEXT,
        reviewed TEXT
    ) STRICT;
    CREATE INDEX review_of_workspace ON review (workspace);
    CREATE UNIQUE INDEX review_of_no_agent ON review (workspace) WHERE agent IS NULL;
    CREATE TABLE thread (
        seq INTEGER PRIMARY KEY,
        id TEXT NOT NULL UNIQUE,
        review TEXT NOT NULL REFERENCES review (id),
        tree TEXT NOT NULL,
        path BLOB NOT NULL,
        side INTEGER NOT NULL CHECK (side IN (0, 1)),
        first_line INTEGER NOT NULL CHECK (first_line >= 1),
        last_line INTEGER NOT NULL CHECK (last_line >= first_line),
        text TEXT NOT NULL,
        resolved INTEGER NOT NULL DEFAULT 0 CHECK (resolved IN (0, 1)),
        addressed INTEGER NOT NULL DEFAULT 0 CHECK (addressed IN (0, 1))
    ) STRICT;
    CREATE INDEX thread_of_review ON thread (review);
    CREATE TABLE comment (
        seq INTEGER PRIMARY KEY,
        id TEXT NOT NULL UNIQUE,
        thread TEXT NOT NULL REFERENCES thread (id),
        author INTEGER NOT NULL CHECK (author IN (0, 1)),
        text TEXT NOT NULL,
        round INTEGER
    ) STRICT;
    CREATE INDEX comment_of_thread ON comment (thread);
    CREATE TABLE round (
        review TEXT NOT NULL REFERENCES review (id),
        number INTEGER NOT NULL CHECK (number >= 1),
        state TEXT NOT NULL,
        sent_by INTEGER NOT NULL CHECK (sent_by IN (0, 1)),
        PRIMARY KEY (review, number)
    ) STRICT;
    CREATE TABLE viewed (
        workspace TEXT NOT NULL,
        path BLOB NOT NULL,
        content TEXT NOT NULL,
        PRIMARY KEY (workspace, path)
    ) STRICT;
    PRAGMA user_version = 2;
";

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
type Job = Box<dyn FnOnce(&mut Connection) + Send>;

/// The database, and the way to its one writer.
///
/// Cheap to clone, and every clone reaches the same writer; it may be used from several
/// threads at once. The writer stops, and the file is closed, when the last clone is
/// dropped: that drop returns when both are done.
#[derive(Debug, Clone)]
pub struct Store {
    // The order of the two fields is the order in which a clone lets go of them: first
    // its way to the writer, then its hold on the writer's thread. The writer ends when
    // no way to it is left, so whoever lets go of the thread last finds it ending.
    writer: mpsc::Sender<Job>,
    _closing: Arc<Closing>,
}

/// Waits for the writer when the last clone of a [`Store`] is dropped.
#[derive(Debug)]
struct Closing {
    writer: Option<JoinHandle<()>>,
}

impl Drop for Closing {
    fn drop(&mut self) {
        if let Some(writer) = self.writer.take() {
            // A writer that panicked has closed the file all the same.
            let _ = writer.join();
        }
    }
}

impl Store {
    /// Opens the database at `file`, making the file and its folder when they are not
    /// there. A database of a newer version than this build knows is refused and left
    /// as it is. One of an older version is copied beside itself, as
    /// `<file>.v<its version>`, and then brought forward to this build's version; a new,
    /// empty file is filled with every version in turn and copied nowhere.
    ///
    /// One process opens a database at a time; that is the Keeper's to see to.
    pub fn open(file: &Path) -> Result<Store, StoreError> {
        let refused = |reason: String| StoreError::Open {
            path: file.to_owned(),
            reason,
        };
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder).map_err(|e| refused(e.to_string()))?;
        }
        let mut connection = Connection::open(file).map_err(|e| refused(e.to_string()))?;

        // The version is read before anything is set: a database of a later version is
        // not touched, not even to change how it is written.
        let found: u32 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|e| refused(e.to_string()))?;
        if found > VERSION {
            return Err(StoreError::Newer {
                found,
                known: VERSION,
            });
        }
        // A change is on the drive when the call that made it returns: a write-ahead
        // log, flushed in full at each transaction. On macOS only `fullfsync` makes the
        // drive empty its own cache; elsewhere the setting does nothing.
        let mode: String = connection
            .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
            .map_err(|e| refused(e.to_string()))?;
        if mode != "wal" {
            return Err(refused(format!(
                "the database is written in the mode {mode}, not with a write-ahead log"
            )));
        }
        connection
            .execute_batch(
                "PRAGMA synchronous = FULL;
                 PRAGMA fullfsync = ON;
                 PRAGMA checkpoint_fullfsync = ON;
                 PRAGMA foreign_keys = ON;",
            )
            .map_err(|e| refused(e.to_string()))?;
        // The tables have had one version so far: there is nothing older to bring
        // forward, only a new file to fill.
        if found < VERSION {
            let filling = connection
                .transaction()
                .map_err(|e| refused(e.to_string()))?;
            filling
                .execute_batch(TABLES)
                .and_then(|()| filling.commit())
                .map_err(|e| refused(e.to_string()))?;
        }

        let (writer, jobs) = mpsc::channel::<Job>();
        let thread = std::thread::Builder::new()
            .name("store-writer".to_owned())
            .spawn(move || {
                // Ends when the last clone of the Store has dropped its sender; the
                // connection closes with it.
                while let Ok(job) = jobs.recv() {
                    job(&mut connection);
                }
            })
            .map_err(|e| refused(e.to_string()))?;
        Ok(Store {
            writer,
            _closing: Arc::new(Closing {
                writer: Some(thread),
            }),
        })
    }

    /// Every repository the user opened, in the order they were first kept.
    pub fn repositories(&self) -> Result<Vec<RepositoryRecord>, StoreError> {
        self.ask(|connection| {
            connection
                .prepare("SELECT id, git_dir FROM repository ORDER BY seq")?
                .query_map([], |row| {
                    Ok(RepositoryRecord {
                        id: RepositoryId(row.get(0)?),
                        git_dir: path(row.get(1)?),
                    })
                })?
                .collect()
        })
    }

    /// Keeps a repository: new when its id is not known, else its git folder is brought
    /// up to date.
    pub fn put_repository(&self, record: &RepositoryRecord) -> Result<(), StoreError> {
        let id = record.id.0.clone();
        let git_dir = bytes(&record.git_dir);
        // Keeping what is kept already changes nothing, and so waits for no drive.
        self.ask(move |connection| {
            connection
                .execute(
                    "INSERT INTO repository (id, git_dir) VALUES (?1, ?2)
                     ON CONFLICT (id) DO UPDATE SET git_dir = excluded.git_dir
                     WHERE git_dir IS NOT excluded.git_dir",
                    (id, git_dir),
                )
                .map(drop)
        })
    }

    /// Every workspace of every repository, in the order they were first kept.
    pub fn workspaces(&self) -> Result<Vec<WorkspaceRecord>, StoreError> {
        self.ask(|connection| {
            connection
                .prepare("SELECT id, repository, path, base FROM workspace ORDER BY seq")?
                .query_map([], |row| {
                    Ok(WorkspaceRecord {
                        id: WorkspaceId(row.get(0)?),
                        repository: RepositoryId(row.get(1)?),
                        path: path(row.get(2)?),
                        base: row.get(3)?,
                    })
                })?
                .collect()
        })
    }

    /// Keeps a workspace: new when its id is not known, else its repository and its
    /// folder are brought up to date. The base the user set for it is left as it is.
    /// The repository has to be kept already.
    pub fn put_workspace(
        &self,
        id: &WorkspaceId,
        repository: &RepositoryId,
        path: &Path,
    ) -> Result<(), StoreError> {
        let (id, repository, path) = (id.0.clone(), repository.0.clone(), bytes(path));
        self.ask(move |connection| {
            connection
                .execute(
                    "INSERT INTO workspace (id, repository, path) VALUES (?1, ?2, ?3)
                     ON CONFLICT (id) DO UPDATE
                     SET repository = excluded.repository, path = excluded.path
                     WHERE repository IS NOT excluded.repository OR path IS NOT excluded.path",
                    (id, repository, path),
                )
                .map(drop)
        })
    }

    /// Sets the base the user chose for a workspace. A workspace the Store does not hold
    /// is left alone.
    pub fn set_base(&self, workspace: &WorkspaceId, base: &str) -> Result<(), StoreError> {
        let (workspace, base) = (workspace.0.clone(), base.to_owned());
        self.ask(move |connection| {
            connection
                .execute(
                    "UPDATE workspace SET base = ?2 WHERE id = ?1 AND base IS NOT ?2",
                    (workspace, base),
                )
                .map(drop)
        })
    }

    /// Hands one piece of work to the writer and waits for what it gives. Each
    /// statement is a transaction of its own.
    fn ask<T, W>(&self, work: W) -> Result<T, StoreError>
    where
        T: Send + 'static,
        W: FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let (answer, answered) = mpsc::channel();
        let job: Job = Box::new(move |connection| {
            // Whoever asked may have stopped waiting; the work is done all the same.
            let _ = answer.send(work(connection));
        });
        self.writer.send(job).map_err(|_| StoreError::Closed)?;
        answered
            .recv()
            .map_err(|_| StoreError::Closed)?
            .map_err(|e| StoreError::Database(e.to_string()))
    }
}

fn bytes(path: &Path) -> Vec<u8> {
    path.as_os_str().as_bytes().to_vec()
}

fn path(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(OsString::from_vec(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    // A unit test that makes a file, because a database of a later version can only be
    // made beside the Store: no call of the Store writes one.
    #[test]
    fn a_database_of_a_later_version_is_refused_and_left_as_it_is() {
        let folder = std::env::temp_dir().join(format!("yaar-store-newer-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("make the folder");
        let file = folder.join(FILE_NAME);
        {
            let later = Connection::open(&file).expect("make the file");
            later
                .execute_batch(&format!(
                    "CREATE TABLE of_a_later_app (what TEXT);
                     INSERT INTO of_a_later_app VALUES ('kept');
                     PRAGMA user_version = {};",
                    VERSION + 1
                ))
                .expect("fill the file");
        }
        let before = std::fs::read(&file).expect("read the file");

        let refused = Store::open(&file);
        let after = std::fs::read(&file).expect("read the file");
        std::fs::remove_dir_all(&folder).expect("remove the folder");

        match refused {
            Err(StoreError::Newer { found, known }) => {
                assert_eq!((found, known), (VERSION + 1, VERSION));
            }
            other => panic!("not refused as a later version: {other:?}"),
        }
        assert_eq!(after, before);
    }
}

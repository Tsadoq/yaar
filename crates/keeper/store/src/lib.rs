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
use std::time::Instant;

use rusqlite::Connection;

mod reviews;
mod sessions;

pub use reviews::{
    Anchor, Author, Comment, CommentId, FLUSH_AFTER, ReviewId, ReviewRecord, Round, SentBy, Side,
    Thread, ThreadId, ViewedRecord,
};
pub use sessions::{SessionId, SessionRecord, StepRecord, TurnEndRecord, TurnId, TurnRecord};

/// The name of the database file in the app's folder.
pub const FILE_NAME: &str = "store.db";

/// The version of the tables this build makes and reads.
const VERSION: u32 = 2;

/// The steps that make the tables, in order: `STEPS[n]` brings a database from version
/// `n` to version `n + 1`, and ends by setting `user_version` to that. A later version
/// adds one step at the end.
const STEPS: [&str; VERSION as usize] = [TABLES, TABLES_2];

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

/// What version 3 adds: the agent sessions, the turns of each and the steps of each turn.
/// A session names its workspace by id, with no foreign key, as a review does; `ended` is
/// the order of ends, a number one more than the largest kept. A turn's `ended` and a
/// step's `kind` are small numbers, in the order their types list them; a step's `what`
/// is a path, as its bytes, or a command, as its text.
///
/// The Store's package applies it as the third step, raises `VERSION` to 3, adds it to
/// `STEPS`, and removes the `expect` below.
#[expect(dead_code, reason = "applied by the Store's package of slice 4")]
const TABLES_3: &str = "
    CREATE TABLE session (
        seq INTEGER PRIMARY KEY,
        id TEXT NOT NULL UNIQUE,
        workspace TEXT NOT NULL,
        agent TEXT NOT NULL,
        cli_session TEXT NOT NULL,
        terminal TEXT,
        number INTEGER NOT NULL CHECK (number >= 1),
        running INTEGER NOT NULL CHECK (running IN (0, 1)),
        ended INTEGER
    ) STRICT;
    CREATE INDEX session_of_workspace ON session (workspace);
    CREATE UNIQUE INDEX session_of_cli ON session (agent, cli_session);
    CREATE TABLE turn (
        seq INTEGER PRIMARY KEY,
        id TEXT NOT NULL UNIQUE,
        session TEXT NOT NULL REFERENCES session (id),
        number INTEGER NOT NULL CHECK (number >= 1),
        ended INTEGER CHECK (ended IN (0, 1)),
        state TEXT
    ) STRICT;
    CREATE INDEX turn_of_session ON turn (session);
    CREATE TABLE step (
        turn TEXT NOT NULL REFERENCES turn (id),
        number INTEGER NOT NULL CHECK (number >= 1),
        kind INTEGER NOT NULL CHECK (kind IN (0, 1, 2, 3)),
        what BLOB NOT NULL,
        PRIMARY KEY (turn, number)
    ) STRICT;
    PRAGMA user_version = 3;
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

/// Why a piece of work on the connection did not give its answer: the database failed,
/// or the Store refused what was asked in a sentence of its own. The database library's
/// error stays inside the crate.
#[derive(Debug)]
enum Failed {
    Sql(rusqlite::Error),
    Refused(String),
}

impl From<rusqlite::Error> for Failed {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sql(e)
    }
}

impl From<Failed> for StoreError {
    fn from(failed: Failed) -> Self {
        match failed {
            Failed::Sql(e) => Self::Database(e.to_string()),
            Failed::Refused(sentence) => Self::Database(sentence),
        }
    }
}

/// What is handed to the writer: one piece of work on what the writer holds.
type Job = Box<dyn FnOnce(&mut Writer) + Send>;

/// What the writer thread holds: the one connection, and since when a draft saved
/// without a flush of the drive waits for one.
struct Writer {
    connection: Connection,
    /// When the last draft was saved that the drive has not been flushed for; none when
    /// there is no such draft.
    unflushed_since: Option<Instant>,
}

impl Writer {
    /// Moves everything the write-ahead file holds into the database file and empties
    /// the file; with `checkpoint_fullfsync` on, the drive is flushed in full for it.
    ///
    /// A checkpoint that a reader of another connection keeps from finishing is not an
    /// error of the database: it answers with a row whose first column is 1. That is a
    /// failure here all the same, and the writer keeps waiting for a flush, so that the
    /// timer tries again.
    fn flush(&mut self) -> Result<(), Failed> {
        // The statement answers with a row, so it is a query.
        let blocked = self
            .connection
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|busy| busy != 0);
        match blocked {
            Ok(false) => {
                self.unflushed_since = None;
                Ok(())
            }
            Ok(true) => {
                self.unflushed_since.get_or_insert_with(Instant::now);
                Err(Failed::Refused(
                    "the write-ahead file could not be moved into the database file".to_owned(),
                ))
            }
            Err(e) => {
                self.unflushed_since.get_or_insert_with(Instant::now);
                Err(e.into())
            }
        }
    }

    /// Runs until the last way to the writer is dropped: each job in turn, and a flush
    /// once no draft was saved for `FLUSH_AFTER`.
    fn run(mut self, jobs: &mpsc::Receiver<Job>) {
        loop {
            let job = match self.unflushed_since {
                None => jobs
                    .recv()
                    .map_err(|_| mpsc::RecvTimeoutError::Disconnected),
                Some(last_save) => {
                    jobs.recv_timeout(FLUSH_AFTER.saturating_sub(last_save.elapsed()))
                }
            };
            match job {
                Ok(job) => job(&mut self),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // A flush that failed is tried again after another pause, not in a
                    // loop.
                    if self.flush().is_err() {
                        self.unflushed_since = Some(Instant::now());
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        // Closing the connection checkpoints too; doing it here keeps the draft's flush
        // from depending on that.
        if self.unflushed_since.is_some() {
            let _ = self.flush();
        }
    }
}

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
        bring_forward(&mut connection, file, found).map_err(|e| refused(e.to_string()))?;

        let (writer, jobs) = mpsc::channel::<Job>();
        let writer_state = Writer {
            connection,
            unflushed_since: None,
        };
        let thread = std::thread::Builder::new()
            .name("store-writer".to_owned())
            .spawn(move || writer_state.run(&jobs))
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
        self.try_ask(move |connection| work(connection).map_err(Failed::Sql))
    }

    /// As [`ask`](Self::ask), for work that may refuse in a sentence of its own.
    fn try_ask<T, W>(&self, work: W) -> Result<T, StoreError>
    where
        T: Send + 'static,
        W: FnOnce(&mut Connection) -> Result<T, Failed> + Send + 'static,
    {
        self.with_writer(move |writer| work(&mut writer.connection))
    }

    /// As [`try_ask`](Self::try_ask), for work that needs what the writer holds besides
    /// the connection.
    fn with_writer<T, W>(&self, work: W) -> Result<T, StoreError>
    where
        T: Send + 'static,
        W: FnOnce(&mut Writer) -> Result<T, Failed> + Send + 'static,
    {
        let (answer, answered) = mpsc::channel();
        let job: Job = Box::new(move |writer| {
            // Whoever asked may have stopped waiting; the work is done all the same.
            let _ = answer.send(work(writer));
        });
        self.writer.send(job).map_err(|_| StoreError::Closed)?;
        answered
            .recv()
            .map_err(|_| StoreError::Closed)?
            .map_err(StoreError::from)
    }
}

/// Brings the tables of a database found at version `found` up to [`VERSION`], one step
/// at a time, each in a transaction. A database of an older version is copied first, so
/// that a step that goes wrong loses nothing.
fn bring_forward(connection: &mut Connection, file: &Path, found: u32) -> Result<(), String> {
    if found > 0 && found < VERSION {
        copy_beside(connection, file, found)?;
    }
    for step in &STEPS[found as usize..] {
        let applying = connection.transaction().map_err(|e| e.to_string())?;
        applying
            .execute_batch(step)
            .and_then(|()| applying.commit())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Copies everything the database holds that is committed, the write-ahead file's
/// content included, to `<file>.v<version>`. The copy is made under another name and
/// renamed once it is on the drive, so that a copy cut short is never taken for a whole
/// one; a copy left by an earlier try is replaced, since the database has not moved on
/// from that version.
fn copy_beside(connection: &Connection, file: &Path, version: u32) -> Result<(), String> {
    let beside = |suffix: String| {
        let mut name = file.as_os_str().to_owned();
        name.push(suffix);
        PathBuf::from(name)
    };
    let copy = beside(format!(".v{version}"));
    let partial = beside(format!(".v{version}.partial"));
    let as_text = partial
        .to_str()
        .ok_or_else(|| format!("the path {} is not text", partial.display()))?;
    match std::fs::remove_file(&partial) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    connection
        .execute("VACUUM INTO ?1", [as_text])
        .map_err(|e| format!("could not copy the database before changing it: {e}"))?;
    std::fs::File::open(&partial)
        .and_then(|copied| copied.sync_all())
        .and_then(|()| std::fs::rename(&partial, &copy))
        .map_err(|e| format!("could not keep the copy {}: {e}", copy.display()))?;
    // The rename is kept by flushing the folder; where a folder cannot be opened it is
    // only as safe as the drive makes it.
    if let Some(folder) = copy.parent() {
        let _ = std::fs::File::open(folder).and_then(|folder| folder.sync_all());
    }
    Ok(())
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

    /// A folder of the test's own, removed when the test ends.
    struct Folder(PathBuf);

    impl Folder {
        fn new(what: &str) -> Self {
            let folder =
                std::env::temp_dir().join(format!("yaar-store-{what}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&folder);
            std::fs::create_dir_all(&folder).expect("make the folder");
            Self(folder)
        }

        fn file(&self) -> PathBuf {
            self.0.join(FILE_NAME)
        }

        fn copy(&self) -> PathBuf {
            self.0.join(format!("{FILE_NAME}.v1"))
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Makes a database of version 1, as the build of slice 1 made them, holding one
    /// repository and two workspaces, and hands back the connection it was made with:
    /// kept open, the rows the connection wrote still lie in the write-ahead file.
    fn version_one(file: &Path) -> Connection {
        let old = Connection::open(file).expect("make the file");
        old.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA wal_autocheckpoint = 0;",
        )
        .expect("set the mode");
        old.execute_batch(TABLES).expect("make the tables");
        old.execute_batch(
            "INSERT INTO repository (id, git_dir) VALUES ('r-1', x'2f722f2e676974');
             INSERT INTO workspace (id, repository, path, base)
                 VALUES ('w-2', 'r-1', x'2f72', 'main'), ('w-1', 'r-1', x'2f722d6c', NULL);",
        )
        .expect("fill the tables");
        old
    }

    fn user_version(file: &Path) -> u32 {
        Connection::open(file)
            .expect("open")
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("read the version")
    }

    fn tables(file: &Path) -> Vec<String> {
        let connection = Connection::open(file).expect("open");
        connection
            .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")
            .expect("prepare")
            .query_map([], |row| row.get(0))
            .expect("query")
            .collect::<rusqlite::Result<_>>()
            .expect("read")
    }

    fn rows(file: &Path) -> Vec<(String, String)> {
        let connection = Connection::open(file).expect("open");
        connection
            .prepare("SELECT id, repository FROM workspace ORDER BY seq")
            .expect("prepare")
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("query")
            .collect::<rusqlite::Result<_>>()
            .expect("read")
    }

    // Unit tests that make a file, because a database of an older version can only be
    // made beside the Store: no call of the Store writes one.
    #[test]
    fn a_database_of_version_1_is_copied_and_then_brought_to_version_2_with_all_it_held() {
        let folder = Folder::new("upgrade");
        let old = version_one(&folder.file());

        let store = Store::open(&folder.file()).expect("open");
        drop(old);
        assert_eq!(
            store.repositories().expect("repositories"),
            vec![RepositoryRecord {
                id: RepositoryId::new("r-1"),
                git_dir: PathBuf::from("/r/.git"),
            }]
        );
        let workspaces = store.workspaces().expect("workspaces");
        assert_eq!(
            workspaces.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
            ["w-2", "w-1"]
        );
        assert_eq!(workspaces[0].base.as_deref(), Some("main"));
        // The tables of version 2 are there to be used.
        store
            .put_review(&ReviewRecord {
                id: ReviewId::new("v"),
                workspace: workspaces[0].id.clone(),
                agent: None,
                reviewed: None,
                threads: vec![],
                rounds: vec![],
            })
            .expect("put review");
        drop(store);

        assert_eq!(user_version(&folder.file()), 2);
        // The copy is of version 1: the rows the write-ahead file held are in it, and
        // none of what version 2 adds.
        assert_eq!(user_version(&folder.copy()), 1);
        assert_eq!(
            rows(&folder.copy()),
            [("w-2".into(), "r-1".into()), ("w-1".into(), "r-1".into())]
        );
        assert_eq!(tables(&folder.copy()), ["repository", "workspace"]);
        assert!(tables(&folder.file()).contains(&"review".to_owned()));
    }

    #[test]
    fn a_second_open_copies_nothing_more_and_a_new_file_is_copied_nowhere() {
        let folder = Folder::new("copies");
        drop(version_one(&folder.file()));
        drop(Store::open(&folder.file()).expect("open"));
        assert!(folder.copy().is_file());

        // Were the copy made again it would be there again.
        std::fs::remove_file(folder.copy()).expect("remove the copy");
        drop(Store::open(&folder.file()).expect("open again"));
        assert!(!folder.copy().exists());

        let fresh = Folder::new("fresh");
        drop(Store::open(&fresh.file()).expect("open"));
        assert_eq!(user_version(&fresh.file()), 2);
        assert_eq!(
            std::fs::read_dir(&fresh.0)
                .expect("list")
                .map(|e| e.expect("entry").file_name())
                .filter(|name| name.to_string_lossy().contains(".v"))
                .count(),
            0
        );
    }

    #[test]
    fn a_step_that_fails_leaves_the_database_at_version_1_with_the_copy_beside_it() {
        let folder = Folder::new("failing");
        let old = version_one(&folder.file());
        // A table of the name version 2 makes: the step cannot be applied.
        old.execute_batch("CREATE TABLE review (what TEXT)")
            .expect("make the table");
        drop(old);

        assert!(matches!(
            Store::open(&folder.file()),
            Err(StoreError::Open { .. })
        ));
        assert_eq!(user_version(&folder.file()), 1);
        assert_eq!(
            tables(&folder.file()),
            ["repository", "review", "workspace"]
        );
        assert_eq!(user_version(&folder.copy()), 1);
        assert_eq!(rows(&folder.copy()).len(), 2);
    }

    #[test]
    fn a_copy_that_an_earlier_try_left_whole_or_cut_short_is_replaced() {
        let folder = Folder::new("stale");
        drop(version_one(&folder.file()));
        std::fs::write(folder.copy(), b"left by a try that stopped").expect("write");
        let mut partial = folder.copy().into_os_string();
        partial.push(".partial");
        std::fs::write(&partial, b"cut short").expect("write");

        drop(Store::open(&folder.file()).expect("open"));
        assert_eq!(user_version(&folder.copy()), 1);
        assert_eq!(rows(&folder.copy()).len(), 2);
        assert!(!Path::new(&partial).exists());
    }

    /// Keeps a review with one thread that holds one draft, and gives the draft's id.
    fn with_one_draft(store: &Store) -> CommentId {
        let review = ReviewId::new("1".repeat(32));
        store
            .put_review(&ReviewRecord {
                id: review.clone(),
                workspace: WorkspaceId::new("2".repeat(32)),
                agent: None,
                reviewed: None,
                threads: vec![],
                rounds: vec![],
            })
            .expect("put review");
        let draft = CommentId::new("4".repeat(32));
        store
            .put_thread(
                &review,
                &Thread {
                    id: ThreadId::new("3".repeat(32)),
                    anchor: Anchor {
                        tree: "a".repeat(40),
                        path: PathBuf::from("a.rs"),
                        side: Side::New,
                        first: 1,
                        last: 1,
                        text: "x".to_owned(),
                    },
                    resolved: false,
                    addressed: false,
                    comments: vec![Comment {
                        id: draft.clone(),
                        author: Author::Reviewer,
                        text: "a".to_owned(),
                        round: None,
                    }],
                },
            )
            .expect("put thread");
        draft
    }

    /// The `synchronous` setting of the writer's connection: 2 is FULL, which every change
    /// but a draft's save waits for the drive with.
    fn synchronous(store: &Store) -> i64 {
        store
            .with_writer(|writer| {
                Ok(writer
                    .connection
                    .query_row("PRAGMA synchronous", [], |row| row.get(0))?)
            })
            .expect("read the setting")
    }

    // Unit tests that hold a second connection open, because no call of the Store holds
    // a read open while the writer works.
    #[test]
    fn a_flush_the_database_could_not_finish_is_a_failure_and_the_writer_tries_again() {
        let folder = Folder::new("blocked");
        let store = Store::open(&folder.file()).expect("open");
        let draft = with_one_draft(&store);

        // A reader that still sees the file as it was before the save keeps the
        // write-ahead file from being emptied: the checkpoint answers with a row that says
        // it was blocked, and not with an error.
        let reader = Connection::open(folder.file()).expect("open a reader");
        reader
            .execute_batch("BEGIN; SELECT count(*) FROM thread;")
            .expect("hold a read");
        store.save_draft(&draft, "typed").expect("save draft");
        assert!(
            store.flush().is_err(),
            "a flush that left the draft in the write-ahead file said it was done"
        );
        let wal = folder.0.join(format!("{FILE_NAME}-wal"));
        assert!(std::fs::metadata(&wal).expect("the write-ahead file").len() > 0);

        // The draft still waits for a flush: once the reader lets go the writer empties
        // the file by itself, with a deadline for the fact.
        reader.execute_batch("COMMIT").expect("let go");
        let deadline = Instant::now() + std::time::Duration::from_secs(30);
        while std::fs::metadata(&wal).expect("the write-ahead file").len() > 0 {
            assert!(Instant::now() < deadline, "the writer never flushed");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    // A unit test, because the setting is the connection's own and only the writer reads
    // it; a kill of the process cannot tell the two settings apart.
    #[test]
    fn a_draft_save_puts_the_wait_for_the_drive_back_whether_or_not_it_changed_a_row() {
        let folder = Folder::new("synchronous");
        let store = Store::open(&folder.file()).expect("open");
        let draft = with_one_draft(&store);
        assert_eq!(synchronous(&store), 2);

        store.save_draft(&draft, "typed").expect("save draft");
        assert_eq!(synchronous(&store), 2, "after a save");
        let unknown = CommentId::new("5".repeat(32));
        assert!(store.save_draft(&unknown, "typed").is_err());
        assert_eq!(synchronous(&store), 2, "after a refused save");
    }

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

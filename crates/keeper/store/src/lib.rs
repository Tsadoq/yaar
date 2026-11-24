//! SQLite database of the Keeper behind one writer thread.
//! A changing call returns once its transaction is on the drive; `Store::save_draft` returns
//! before, and the writer flushes when typing pauses.

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
mod settings;

pub use reviews::{
    Anchor, AttachedRecord, Author, Comment, CommentId, CreditRecord, FLUSH_AFTER, NoticeRecord,
    ReviewId, ReviewRecord, Round, SentBy, Side, Thread, ThreadId, ThreadPlace, ViewedRecord,
};
pub use sessions::{
    EndedBy, SessionId, SessionRecord, StepRecord, TurnEndRecord, TurnId, TurnRecord,
};
pub use settings::{FlowForm, OptionKey, Setting, TerminalPlace, Theme, TurnEndSelects};

pub const FILE_NAME: &str = "store.db";

const VERSION: u32 = 7;

const STEPS: [&str; VERSION as usize] = [
    TABLES, TABLES_2, TABLES_3, TABLES_4, TABLES_5, TABLES_6, TABLES_7,
];

/// `seq` is the order in which rows were first kept. Paths are kept as bytes, not text.
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

/// Side, author and way of sending are small numbers in the order their types list them.
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

/// `ended` orders session ends (largest kept + 1). `ended_by`, a turn's `ended` and a step's
/// `kind` are numbers in the order of their types. `what` is path bytes or command text.
/// `lines` is `a-b` ranges joined by `,`: empty for an edit that named none, null if none said.
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
        ended INTEGER,
        ended_by INTEGER CHECK (ended_by IN (0, 1, 2))
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
        lines TEXT,
        PRIMARY KEY (turn, number)
    ) STRICT;
    PRAGMA user_version = 3;
";

/// Turns kept before this version have a null `start`.
const TABLES_4: &str = "
    ALTER TABLE turn ADD COLUMN start TEXT;
    CREATE TABLE setting (
        name TEXT PRIMARY KEY,
        value TEXT NOT NULL
    ) STRICT;
    CREATE TABLE workspace_setting (
        workspace TEXT NOT NULL,
        name TEXT NOT NULL,
        value TEXT NOT NULL,
        PRIMARY KEY (workspace, name)
    ) STRICT;
    PRAGMA user_version = 4;
";

/// SQLite cannot drop a column's `NOT NULL`, so `round` is rebuilt. `attached` is null for
/// nothing, 0 for the review, 1 for one thread (`attached_thread`). In `state_credit` the
/// session `''` is a change no agent made.
const TABLES_5: &str = "
    CREATE TABLE round_5 (
        review TEXT NOT NULL REFERENCES review (id),
        number INTEGER NOT NULL CHECK (number >= 1),
        state TEXT,
        sent_by INTEGER NOT NULL CHECK (sent_by IN (0, 1)),
        edited INTEGER NOT NULL DEFAULT 0 CHECK (edited IN (0, 1)),
        PRIMARY KEY (review, number)
    ) STRICT;
    INSERT INTO round_5 (review, number, state, sent_by)
        SELECT review, number, state, sent_by FROM round;
    DROP TABLE round;
    ALTER TABLE round_5 RENAME TO round;
    ALTER TABLE thread ADD COLUMN now_tree TEXT;
    ALTER TABLE thread ADD COLUMN now_path BLOB;
    ALTER TABLE thread ADD COLUMN now_side INTEGER CHECK (now_side IN (0, 1));
    ALTER TABLE thread ADD COLUMN now_first INTEGER CHECK (now_first >= 1);
    ALTER TABLE thread ADD COLUMN now_last INTEGER;
    ALTER TABLE thread ADD COLUMN notice INTEGER CHECK (notice IN (0, 1));
    ALTER TABLE review ADD COLUMN attached INTEGER CHECK (attached IN (0, 1));
    ALTER TABLE review ADD COLUMN attached_thread TEXT;
    CREATE TABLE state_credit (
        workspace TEXT NOT NULL,
        state TEXT NOT NULL,
        path BLOB NOT NULL,
        session TEXT NOT NULL,
        PRIMARY KEY (workspace, state, path, session)
    ) STRICT;
    PRAGMA user_version = 5;
";

/// `state_kept` tells a state kept with no credits (a worktree with nothing uncommitted) from
/// one nothing was kept for.
const TABLES_6: &str = "
    ALTER TABLE session ADD COLUMN cli_record BLOB;
    ALTER TABLE round ADD COLUMN turn TEXT;
    CREATE TABLE state_kept (
        workspace TEXT NOT NULL,
        state TEXT NOT NULL,
        PRIMARY KEY (workspace, state)
    ) STRICT;
    INSERT OR IGNORE INTO state_kept (workspace, state)
        SELECT DISTINCT workspace, state FROM state_credit;
    PRAGMA user_version = 6;
";

const TABLES_7: &str = "
    CREATE TABLE repository_setting (
        repository TEXT NOT NULL,
        name TEXT NOT NULL,
        value TEXT NOT NULL,
        PRIMARY KEY (repository, name)
    ) STRICT;
    PRAGMA user_version = 7;
";

/// 32 lowercase hex digits, drawn at random when the worktree is first seen.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkspaceId(String);

impl WorkspaceId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepositoryId(String);

impl RepositoryId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RepositoryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryRecord {
    pub id: RepositoryId,
    pub git_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRecord {
    pub id: WorkspaceId,
    pub repository: RepositoryId,
    pub path: PathBuf,
    pub base: Option<String>,
}

#[derive(Debug)]
pub enum StoreError {
    Open { path: PathBuf, reason: String },
    Newer { found: u32, known: u32 },
    Database(String),
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

type Job = Box<dyn FnOnce(&mut Writer) + Send>;

struct Writer {
    connection: Connection,
    unflushed_since: Option<Instant>,
}

impl Writer {
    /// A checkpoint that a reader of another connection keeps from finishing answers with a row
    /// whose first column is 1, not an error. That is a failure here, so the timer retries.
    fn flush(&mut self) -> Result<(), Failed> {
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
                    // A failed flush is retried after another pause, not in a loop.
                    if self.flush().is_err() {
                        self.unflushed_since = Some(Instant::now());
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        // Closing the connection checkpoints too; doing it here keeps the draft's flush from
        // depending on that.
        if self.unflushed_since.is_some() {
            let _ = self.flush();
        }
    }
}

/// Cheap to clone; clones share one writer. The last drop stops the writer and closes the file.
#[derive(Debug, Clone)]
pub struct Store {
    // Field order is drop order: the way to the writer first, then the thread handle, so whoever
    // lets go last finds the writer ending.
    writer: mpsc::Sender<Job>,
    _closing: Arc<Closing>,
}

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
    /// A newer version is refused and left as it is. An older one is copied to
    /// `<file>.v<its version>` and brought forward; a new file is copied nowhere.
    pub fn open(file: &Path) -> Result<Store, StoreError> {
        let refused = |reason: String| StoreError::Open {
            path: file.to_owned(),
            reason,
        };
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder).map_err(|e| refused(e.to_string()))?;
        }
        let mut connection = Connection::open(file).map_err(|e| refused(e.to_string()))?;

        // Read before anything is set, so a newer database is not touched at all.
        let found: u32 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|e| refused(e.to_string()))?;
        if found > VERSION {
            return Err(StoreError::Newer {
                found,
                known: VERSION,
            });
        }
        // A write-ahead log flushed in full at each transaction. Only `fullfsync` makes the drive
        // empty its cache on macOS; elsewhere the setting does nothing.
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

    /// Leaves the base the user set as it is.
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

    fn ask<T, W>(&self, work: W) -> Result<T, StoreError>
    where
        T: Send + 'static,
        W: FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        self.try_ask(move |connection| work(connection).map_err(Failed::Sql))
    }

    fn try_ask<T, W>(&self, work: W) -> Result<T, StoreError>
    where
        T: Send + 'static,
        W: FnOnce(&mut Connection) -> Result<T, Failed> + Send + 'static,
    {
        self.with_writer(move |writer| work(&mut writer.connection))
    }

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

/// Copies the committed content, write-ahead file included, to `<file>.v<version>.partial`,
/// then renames it to `<file>.v<version>` once on the drive: a cut-short copy is never taken
/// for a whole one.
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
    // The rename is kept by flushing the folder; where a folder cannot be opened it is only as
    // safe as the drive makes it.
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

    #[test]
    fn a_database_of_version_1_is_copied_and_then_brought_to_version_7_with_all_it_held() {
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
        store
            .put_review(&ReviewRecord {
                id: ReviewId::new("v"),
                workspace: workspaces[0].id.clone(),
                agent: None,
                reviewed: None,
                threads: vec![],
                rounds: vec![],
                attached: None,
            })
            .expect("put review");
        drop(store);

        assert_eq!(user_version(&folder.file()), 7);
        assert_eq!(user_version(&folder.copy()), 1);
        assert_eq!(
            rows(&folder.copy()),
            [("w-2".into(), "r-1".into()), ("w-1".into(), "r-1".into())]
        );
        assert_eq!(tables(&folder.copy()), ["repository", "workspace"]);
        assert!(tables(&folder.file()).contains(&"review".to_owned()));
    }

    fn version_two(file: &Path) -> Connection {
        let old = version_one(file);
        old.execute_batch(TABLES_2).expect("make the tables of 2");
        old.execute_batch(
            "INSERT INTO review (id, workspace, agent, reviewed)
                 VALUES ('v-1', 'w-1', NULL, 'abc');",
        )
        .expect("fill the review");
        old
    }

    #[test]
    fn a_database_of_version_2_is_copied_and_then_brought_to_version_7_with_its_reviews() {
        let folder = Folder::new("upgrade-two");
        let old = version_two(&folder.file());
        let copy = folder.0.join(format!("{FILE_NAME}.v2"));

        let store = Store::open(&folder.file()).expect("open");
        drop(old);
        let kept = store
            .reviews_of(&WorkspaceId::new("w-1"))
            .expect("reviews of the workspace");
        assert_eq!(
            kept.iter()
                .map(|r| (r.id.as_str(), r.agent.as_deref(), r.reviewed.as_deref()))
                .collect::<Vec<_>>(),
            [("v-1", None, Some("abc"))]
        );
        let session = SessionRecord {
            id: SessionId::new("s-1"),
            workspace: WorkspaceId::new("w-1"),
            agent: "claude".to_owned(),
            cli_session: "cli-1".to_owned(),
            terminal: None,
            number: 1,
            running: true,
            ended: None,
            ended_by: None,
        };
        store.put_session(&session).expect("put session");
        assert_eq!(store.session(&session.id).expect("session"), Some(session));
        drop(store);

        assert_eq!(user_version(&folder.file()), 7);
        assert_eq!(
            tables(&folder.file()),
            [
                "comment",
                "repository",
                "repository_setting",
                "review",
                "round",
                "session",
                "setting",
                "state_credit",
                "state_kept",
                "step",
                "thread",
                "turn",
                "viewed",
                "workspace",
                "workspace_setting"
            ]
        );
        assert_eq!(user_version(&copy), 2);
        assert!(!folder.copy().exists());
        assert!(tables(&copy).contains(&"review".to_owned()));
        assert!(!tables(&copy).contains(&"session".to_owned()));
        let held: String = Connection::open(&copy)
            .expect("open the copy")
            .query_row("SELECT reviewed FROM review WHERE id = 'v-1'", [], |row| {
                row.get(0)
            })
            .expect("the review is in the copy");
        assert_eq!(held, "abc");
    }

    fn version_three(file: &Path) -> Connection {
        let old = version_two(file);
        old.execute_batch(TABLES_3).expect("make the tables of 3");
        old.execute_batch(
            "INSERT INTO session (id, workspace, agent, cli_session, terminal, number, running)
                 VALUES ('s-1', 'w-1', 'claude', 'cli-1', 't1', 1, 1);
             INSERT INTO turn (id, session, number, ended, state)
                 VALUES ('t-1', 's-1', 1, 0, 'aaa'), ('t-2', 's-1', 2, NULL, NULL);
             INSERT INTO step (turn, number, kind, what, lines)
                 VALUES ('t-1', 1, 0, x'612e747874', NULL), ('t-1', 2, 1, x'612e747874', '3-4,9-9');",
        )
        .expect("fill the session");
        old
    }

    #[test]
    fn a_database_of_version_3_is_copied_and_then_brought_to_version_7_with_its_sessions_and_turns()
    {
        let folder = Folder::new("upgrade-three");
        let old = version_three(&folder.file());
        let copy = folder.0.join(format!("{FILE_NAME}.v3"));

        let store = Store::open(&folder.file()).expect("open");
        drop(old);
        let turns = store.turns_of(&WorkspaceId::new("w-1")).expect("turns");
        assert_eq!(
            turns
                .iter()
                .map(|t| (
                    t.id.as_str(),
                    t.number,
                    t.ended,
                    t.state.as_deref(),
                    &t.start
                ))
                .collect::<Vec<_>>(),
            [
                ("t-1", 1, Some(TurnEndRecord::Finished), Some("aaa"), &None),
                ("t-2", 2, None, None, &None),
            ]
        );
        let (_, steps) = store
            .turn(&TurnId::new("t-1"))
            .expect("turn")
            .expect("kept");
        assert_eq!(
            steps,
            [
                StepRecord::Read(PathBuf::from("a.txt")),
                StepRecord::Edit(PathBuf::from("a.txt"), Some(vec![3..=4, 9..=9])),
            ]
        );
        assert_eq!(
            store
                .session(&SessionId::new("s-1"))
                .expect("session")
                .map(|s| (s.cli_session, s.running)),
            Some(("cli-1".to_owned(), true))
        );
        store
            .set_turn_start(&TurnId::new("t-2"), &"f".repeat(40))
            .expect("set the start");
        store
            .set_setting(&Setting::Theme(Theme::Light))
            .expect("set a setting");
        assert_eq!(
            store
                .turn(&TurnId::new("t-2"))
                .expect("turn")
                .expect("kept")
                .0
                .start,
            Some("f".repeat(40))
        );
        drop(store);

        assert_eq!(user_version(&folder.file()), 7);
        assert_eq!(user_version(&copy), 3);
        assert!(!folder.copy().exists());
        assert!(!folder.0.join(format!("{FILE_NAME}.v2")).exists());
        assert!(!tables(&copy).contains(&"setting".to_owned()));
        assert!(tables(&copy).contains(&"step".to_owned()));
        let held: (i64, Option<String>) = Connection::open(&copy)
            .expect("open the copy")
            .query_row(
                "SELECT (SELECT COUNT(*) FROM turn), (SELECT state FROM turn WHERE id = 't-1')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("the turns are in the copy");
        assert_eq!(held, (2, Some("aaa".to_owned())));
        assert!(
            Connection::open(&copy)
                .expect("open the copy")
                .prepare("SELECT start FROM turn")
                .is_err(),
            "the copy has no column of version 4"
        );
    }

    fn version_four(file: &Path) -> Connection {
        let old = version_three(file);
        old.execute_batch(TABLES_4).expect("make the tables of 4");
        old.execute_batch(
            "UPDATE turn SET start = 'bbb' WHERE id = 't-1';
             INSERT INTO thread
                 (id, review, tree, path, side, first_line, last_line, text, resolved, addressed)
                 VALUES ('th-1', 'v-1', 'ccc', x'612e7273', 1, 3, 4, 'let x', 0, 1),
                        ('th-2', 'v-1', 'ccc', x'622e7273', 0, 1, 1, 'old', 1, 0);
             INSERT INTO comment (id, thread, author, text, round)
                 VALUES ('c-1', 'th-1', 0, 'fix this', 1), ('c-2', 'th-1', 1, 'done', 1),
                        ('c-3', 'th-2', 0, 'a draft', NULL);
             INSERT INTO round (review, number, state, sent_by)
                 VALUES ('v-1', 1, 'ddd', 1), ('v-1', 2, 'eee', 0);
             INSERT INTO setting (name, value) VALUES ('theme', 'dark');
             INSERT INTO workspace_setting (workspace, name, value)
                 VALUES ('w-1', 'flow_form', 'live_diff');",
        )
        .expect("fill the reviews and the settings");
        old
    }

    #[test]
    fn a_database_of_version_4_is_copied_and_then_brought_to_version_7_with_all_it_held() {
        let folder = Folder::new("upgrade-four");
        let old = version_four(&folder.file());
        let copy = folder.0.join(format!("{FILE_NAME}.v4"));

        let store = Store::open(&folder.file()).expect("open");
        drop(old);
        let reviews = store.reviews_of(&WorkspaceId::new("w-1")).expect("reviews");
        let [review] = reviews.as_slice() else {
            panic!("one review was kept: {reviews:?}");
        };
        assert_eq!(
            review.rounds,
            [
                Round {
                    number: 1,
                    state: Some("ddd".to_owned()),
                    sent_by: SentBy::Delivered,
                    edited: false,
                },
                Round {
                    number: 2,
                    state: Some("eee".to_owned()),
                    sent_by: SentBy::Copied,
                    edited: false,
                },
            ]
        );
        assert_eq!(review.attached, None);
        assert_eq!(review.reviewed.as_deref(), Some("abc"));
        assert_eq!(
            review
                .threads
                .iter()
                .map(|t| (t.id.as_str(), t.resolved, t.addressed, t.now.clone()))
                .collect::<Vec<_>>(),
            [("th-1", false, true, None), ("th-2", true, false, None)]
        );
        assert_eq!(
            review
                .threads
                .iter()
                .map(|t| t
                    .comments
                    .iter()
                    .map(|c| (c.id.as_str(), c.author, c.text.as_str(), c.round))
                    .collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            [
                vec![
                    ("c-1", Author::Reviewer, "fix this", Some(1)),
                    ("c-2", Author::Agent, "done", Some(1)),
                ],
                vec![("c-3", Author::Reviewer, "a draft", None)],
            ]
        );
        let turns = store.turns_of(&WorkspaceId::new("w-1")).expect("turns");
        assert_eq!(
            turns
                .iter()
                .map(|t| (t.id.as_str(), t.start.as_deref(), t.state.as_deref()))
                .collect::<Vec<_>>(),
            [("t-1", Some("bbb"), Some("aaa")), ("t-2", None, None)]
        );
        let (_, steps) = store
            .turn(&TurnId::new("t-1"))
            .expect("turn")
            .expect("kept");
        assert_eq!(steps.len(), 2);
        assert_eq!(
            store
                .session(&SessionId::new("s-1"))
                .expect("session")
                .map(|s| s.cli_session),
            Some("cli-1".to_owned())
        );
        assert_eq!(
            store.settings().expect("settings"),
            [
                Setting::Theme(Theme::Dark),
                Setting::FlowForm {
                    workspace: WorkspaceId::new("w-1"),
                    form: FlowForm::LiveDiff,
                },
            ]
        );
        store
            .set_round_edited(&review.id, 2)
            .expect("set the edit mark");
        store
            .set_attached(&review.id, Some(&AttachedRecord::Review))
            .expect("attach");
        store
            .put_credits(
                &WorkspaceId::new("w-1"),
                "eee",
                &[CreditRecord {
                    path: PathBuf::from("a.rs"),
                    sessions: vec![],
                }],
            )
            .expect("put credits");
        let reviewed = store.review(&review.id).expect("review").expect("kept");
        assert_eq!(
            reviewed.rounds.iter().map(|r| r.edited).collect::<Vec<_>>(),
            [false, true]
        );
        assert_eq!(reviewed.attached, Some(AttachedRecord::Review));
        drop(store);

        assert_eq!(user_version(&folder.file()), 7);
        assert_eq!(user_version(&copy), 4);
        for earlier in 1..4 {
            assert!(!folder.0.join(format!("{FILE_NAME}.v{earlier}")).exists());
        }
        assert!(!tables(&copy).contains(&"state_credit".to_owned()));
        let held: (i64, i64, i64, i64, i64, String) = Connection::open(&copy)
            .expect("open the copy")
            .query_row(
                "SELECT (SELECT COUNT(*) FROM round), (SELECT COUNT(*) FROM thread),
                        (SELECT COUNT(*) FROM comment), (SELECT COUNT(*) FROM turn),
                        (SELECT COUNT(*) FROM setting),
                        (SELECT state FROM round WHERE number = 2)",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .expect("everything is in the copy");
        assert_eq!(held, (2, 2, 3, 2, 1, "eee".to_owned()));
        assert!(
            Connection::open(&copy)
                .expect("open the copy")
                .prepare("SELECT edited FROM round")
                .is_err(),
            "the copy has no column of version 5"
        );
    }

    fn version_five(file: &Path) -> Connection {
        let old = version_four(file);
        old.execute_batch(TABLES_5).expect("make the tables of 5");
        old.execute_batch(
            "INSERT INTO state_credit (workspace, state, path, session)
                 VALUES ('w-1', 'eee', x'612e7273', 's-1'), ('w-1', 'eee', x'622e7273', ''),
                        ('w-1', 'fff', x'612e7273', ''), ('w-2', 'eee', x'612e7273', '');
             UPDATE round SET edited = 1 WHERE number = 2;",
        )
        .expect("fill the credits");
        old
    }

    #[test]
    fn a_database_of_version_5_is_copied_and_then_brought_to_version_7_with_all_it_held() {
        let folder = Folder::new("upgrade-five");
        let old = version_five(&folder.file());
        let copy = folder.0.join(format!("{FILE_NAME}.v5"));

        let store = Store::open(&folder.file()).expect("open");
        drop(old);
        let review = ReviewId::new("v-1");
        let kept = store.review(&review).expect("review").expect("kept");
        assert_eq!(
            kept.rounds
                .iter()
                .map(|r| (r.number, r.state.as_deref(), r.edited))
                .collect::<Vec<_>>(),
            [(1, Some("ddd"), false), (2, Some("eee"), true)]
        );
        assert_eq!(store.round_turns(&review).expect("turns"), []);
        assert_eq!(kept.threads.len(), 2);
        assert_eq!(
            store
                .turns_of(&WorkspaceId::new("w-1"))
                .expect("turns")
                .len(),
            2
        );
        assert_eq!(
            store.settings().expect("settings").len(),
            2,
            "the settings are kept"
        );
        let w1 = WorkspaceId::new("w-1");
        assert_eq!(
            store.credits_of(&w1, "eee").expect("credits"),
            [
                CreditRecord {
                    path: PathBuf::from("a.rs"),
                    sessions: vec!["s-1".to_owned()],
                },
                CreditRecord {
                    path: PathBuf::from("b.rs"),
                    sessions: vec![],
                },
            ]
        );
        for (workspace, state, expected) in [
            ("w-1", "eee", true),
            ("w-1", "fff", true),
            ("w-2", "eee", true),
            ("w-1", "ddd", false),
            ("w-2", "fff", false),
        ] {
            assert_eq!(
                store
                    .credits_were_kept(&WorkspaceId::new(workspace), state)
                    .expect("kept"),
                expected,
                "{workspace} {state}"
            );
        }
        let session = SessionId::new("s-1");
        assert_eq!(store.session_cli_record(&session).expect("record"), None);
        store
            .set_session_cli_record(&session, Some(Path::new("/s/one.jsonl")))
            .expect("set the record");
        store
            .set_round_turn(&review, 2, "t-2")
            .expect("set the turn");
        assert_eq!(
            store.session_cli_record(&session).expect("record"),
            Some(PathBuf::from("/s/one.jsonl"))
        );
        assert_eq!(
            store.round_turns(&review).expect("turns"),
            [(2, "t-2".to_owned())]
        );
        drop(store);

        assert_eq!(user_version(&folder.file()), 7);
        assert_eq!(user_version(&copy), 5);
        for earlier in 1..5 {
            assert!(!folder.0.join(format!("{FILE_NAME}.v{earlier}")).exists());
        }
        assert!(!tables(&copy).contains(&"state_kept".to_owned()));
        let copied = Connection::open(&copy).expect("open the copy");
        let held: (i64, i64, i64, i64) = copied
            .query_row(
                "SELECT (SELECT COUNT(*) FROM state_credit), (SELECT COUNT(*) FROM round),
                        (SELECT COUNT(*) FROM session), (SELECT COUNT(*) FROM setting)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("everything is in the copy");
        assert_eq!(held, (4, 2, 1, 1));
        assert!(
            copied.prepare("SELECT cli_record FROM session").is_err(),
            "the copy has no column of version 6"
        );
        assert!(
            copied.prepare("SELECT turn FROM round").is_err(),
            "the copy has no column of version 6"
        );
    }

    fn version_six(file: &Path) -> Connection {
        let old = version_five(file);
        old.execute_batch(TABLES_6).expect("make the tables of 6");
        old.execute_batch(
            "UPDATE session SET cli_record = x'2f732f6f6e652e6a736f6e6c' WHERE id = 's-1';
             UPDATE round SET turn = 't-2' WHERE number = 2;
             INSERT INTO viewed (workspace, path, content)
                 VALUES ('w-1', x'612e7273', 'abc'), ('w-2', x'622e7273', 'def');",
        )
        .expect("fill what version 6 added");
        old
    }

    #[test]
    fn a_database_of_version_6_is_copied_and_then_brought_to_version_7_with_all_it_held() {
        let folder = Folder::new("upgrade-six");
        let old = version_six(&folder.file());
        let copy = folder.0.join(format!("{FILE_NAME}.v6"));

        let store = Store::open(&folder.file()).expect("open");
        drop(old);
        assert_eq!(
            store.settings().expect("settings"),
            [
                Setting::Theme(Theme::Dark),
                Setting::FlowForm {
                    workspace: WorkspaceId::new("w-1"),
                    form: FlowForm::LiveDiff,
                },
            ]
        );
        assert_eq!(
            store
                .session_cli_record(&SessionId::new("s-1"))
                .expect("record"),
            Some(PathBuf::from("/s/one.jsonl"))
        );
        let review = ReviewId::new("v-1");
        assert_eq!(
            store.round_turns(&review).expect("turns"),
            [(2, "t-2".to_owned())]
        );
        let w1 = WorkspaceId::new("w-1");
        assert_eq!(store.credits_of(&w1, "eee").expect("credits").len(), 2);
        assert!(store.credits_were_kept(&w1, "fff").expect("kept"));
        let answer = Setting::ServersInRepository {
            repository: RepositoryId::new("r-1"),
            allowed: true,
        };
        store.set_setting(&answer).expect("set");
        assert_eq!(
            store.settings().expect("settings").last(),
            Some(&answer),
            "the answer on a repository is kept in a file brought forward"
        );
        drop(store);

        assert_eq!(user_version(&folder.file()), 7);
        assert_eq!(user_version(&copy), 6);
        for earlier in 1..6 {
            assert!(!folder.0.join(format!("{FILE_NAME}.v{earlier}")).exists());
        }
        assert!(!tables(&copy).contains(&"repository_setting".to_owned()));
        let copied = Connection::open(&copy).expect("open the copy");
        let held: Vec<i64> = [
            "repository",
            "workspace",
            "session",
            "round",
            "state_credit",
            "state_kept",
            "setting",
            "workspace_setting",
        ]
        .iter()
        .map(|table| {
            copied
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .expect("everything is in the copy")
        })
        .collect();
        assert_eq!(held, [1, 2, 1, 2, 4, 3, 1, 1]);
        let record: Vec<u8> = copied
            .query_row("SELECT cli_record FROM session", [], |row| row.get(0))
            .expect("the copy holds the record of version 6");
        assert_eq!(record, b"/s/one.jsonl");

        let upgraded = Connection::open(folder.file()).expect("open the upgraded file");
        let names = tables(&copy);
        assert_eq!(
            names,
            [
                "comment",
                "repository",
                "review",
                "round",
                "session",
                "setting",
                "state_credit",
                "state_kept",
                "step",
                "thread",
                "turn",
                "viewed",
                "workspace",
                "workspace_setting"
            ]
        );
        let mut filled = Vec::new();
        for name in &names {
            let before = rows_of(&copied, name);
            if !before.is_empty() {
                filled.push(name.as_str());
            }
            assert_eq!(rows_of(&upgraded, name), before, "the rows of {name}");
        }
        assert_eq!(
            filled,
            [
                "comment",
                "repository",
                "review",
                "round",
                "session",
                "setting",
                "state_credit",
                "state_kept",
                "step",
                "thread",
                "turn",
                "viewed",
                "workspace",
                "workspace_setting"
            ],
            "the tables version 6 holds rows of"
        );
    }

    fn rows_of(connection: &Connection, table: &str) -> Vec<String> {
        let columns: Vec<String> = connection
            .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
            .expect("the columns")
            .query_map([], |row| row.get(0))
            .expect("read the columns")
            .collect::<Result<_, _>>()
            .expect("the names");
        let line = columns
            .iter()
            .map(|column| format!("quote(\"{column}\")"))
            .collect::<Vec<_>>()
            .join(" || '|' || ");
        let mut rows: Vec<String> = connection
            .prepare(&format!("SELECT {line} FROM \"{table}\""))
            .expect("the rows")
            .query_map([], |row| row.get(0))
            .expect("read the rows")
            .collect::<Result<_, _>>()
            .expect("the lines");
        rows.sort();
        rows
    }

    #[test]
    fn a_second_open_copies_nothing_more_and_a_new_file_is_copied_nowhere() {
        let folder = Folder::new("copies");
        drop(version_one(&folder.file()));
        drop(Store::open(&folder.file()).expect("open"));
        assert!(folder.copy().is_file());

        std::fs::remove_file(folder.copy()).expect("remove the copy");
        drop(Store::open(&folder.file()).expect("open again"));
        assert!(!folder.copy().exists());

        let fresh = Folder::new("fresh");
        drop(Store::open(&fresh.file()).expect("open"));
        assert_eq!(user_version(&fresh.file()), 7);
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
                attached: None,
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
                    now: None,
                },
            )
            .expect("put thread");
        draft
    }

    /// 2 is FULL, which every change but a draft's save waits for the drive with.
    fn synchronous(store: &Store) -> i64 {
        store
            .with_writer(|writer| {
                Ok(writer
                    .connection
                    .query_row("PRAGMA synchronous", [], |row| row.get(0))?)
            })
            .expect("read the setting")
    }

    // Hold a second connection: no call of the Store holds a read open while the writer works.
    #[test]
    fn a_flush_the_database_could_not_finish_is_a_failure_and_the_writer_tries_again() {
        let folder = Folder::new("blocked");
        let store = Store::open(&folder.file()).expect("open");
        let draft = with_one_draft(&store);

        // A reader still seeing the file as before the save keeps the write-ahead file from
        // being emptied: the checkpoint answers with a row saying it was blocked, not an error.
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

        reader.execute_batch("COMMIT").expect("let go");
        let deadline = Instant::now() + std::time::Duration::from_secs(30);
        while std::fs::metadata(&wal).expect("the write-ahead file").len() > 0 {
            assert!(Instant::now() < deadline, "the writer never flushed");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    // `synchronous` belongs to the connection, so only the writer can read it; a kill test cannot
    // tell NORMAL from FULL, since a killed process loses nothing under either.
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

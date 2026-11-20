//! Component tests: what a Window reads of the agents' side that is no agent, through the
//! Link: one turn, the history of a worktree, the uncommitted changes a review lists, and
//! two versions of a change. A real Keeper runs in this process on a database, a folder of
//! state stores and repositories the test makes. What the Keeper keeps about sessions and
//! turns, and the states of the worktrees, the test puts there with the Store's and the
//! engine's own calls before the Keeper starts: no hook program runs.
//!
//! The Keeper runs git as its user does, with the environment of its process. Each test
//! therefore cleans that environment as its first statement, and these tests are run by
//! `cargo nextest`, which gives each test a process of its own.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_keeper::{Exit, Options, run};
use yaar_link::{
    AgentReport, ChangedFile, Commit, Content, CreditedFile, Fact, FileSides, FileStatus, Hello,
    History, HistoryEntry, MAX_CONTENT, Message, Program, Refusal, Reply, Request, RequestId,
    ReviewId, SessionId, Step, TerminalId, TerminalSize, Turn, TurnEnd, TurnId, WorkspaceId,
    handshake, read_message, write_message,
};
use yaar_recorder::{Git, Worktree};
use yaar_review_engine::Reviews;
use yaar_store::{EndedBy, SessionRecord, StepRecord, Store, TurnEndRecord, TurnRecord};
use yaar_workspaces::Workspaces;

/// How long a test waits for the Keeper before it fails.
const DEADLINE: Duration = Duration::from_secs(20);

/// A folder of the test's own with a short path, removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    /// Makes the folder and cleans the environment of this process for git: no
    /// configuration of the user or of the system, a fixed author and date, an empty
    /// home. It has to be the first statement of a test.
    fn new() -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        // `/tmp` is a link on macOS, and the Keeper names a worktree by its resolved path.
        let root = Path::new("/tmp")
            .canonicalize()
            .expect("the temporary folder")
            .join(format!("yaar-keeper-reading-{}-{n}", std::process::id()));
        for folder in ["home", "tmp"] {
            std::fs::create_dir_all(root.join(folder)).expect("make the test's folder");
        }
        let settings = [
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_AUTHOR_NAME", "A Test"),
            ("GIT_AUTHOR_EMAIL", "test@example.invalid"),
            ("GIT_AUTHOR_DATE", "2026-01-01T12:00:00Z"),
            ("GIT_COMMITTER_NAME", "A Test"),
            ("GIT_COMMITTER_EMAIL", "test@example.invalid"),
            ("GIT_COMMITTER_DATE", "2026-01-01T12:00:00Z"),
            // The shell of a terminal the test opens.
            ("SHELL", "/bin/sh"),
        ];
        // SAFETY: changing the environment is sound only while no other thread reads it,
        // as a thread that starts a process does. This runs as the first statement of a
        // test, and these tests are run by `cargo nextest`, which gives each test a
        // process of its own: no other thread exists yet.
        unsafe {
            for (name, value) in settings {
                std::env::set_var(name, value);
            }
            std::env::set_var("HOME", root.join("home"));
            // Where the Keeper's copies of git's lists of files go: a folder that goes
            // with the test.
            std::env::set_var("TMPDIR", root.join("tmp"));
            for name in [
                "GIT_DIR",
                "GIT_WORK_TREE",
                "GIT_INDEX_FILE",
                "GIT_CONFIG_COUNT",
                "GIT_CONFIG_PARAMETERS",
                "GIT_CONFIG",
            ] {
                std::env::remove_var(name);
            }
        }
        Scratch(root)
    }

    fn socket(&self) -> PathBuf {
        self.0.join("k.sock")
    }

    /// The app's folder of the Keeper of this test.
    fn data(&self) -> PathBuf {
        self.0.join("data")
    }

    /// Starts the Keeper's library on a thread of this test.
    fn keeper(&self) -> JoinHandle<std::io::Result<Exit>> {
        let options = Options {
            socket: self.socket(),
            linger: Duration::from_secs(60),
            app_version: "test-keeper".to_owned(),
            data_dir: self.data(),
            companions: None,
            home: self.0.join("home"),
        };
        std::thread::spawn(move || run(options))
    }

    /// Attaches as a Window does, waiting for the Keeper to listen.
    fn window(&self) -> Window {
        let mine = Hello::for_this_process("test-window");
        let deadline = Instant::now() + DEADLINE;
        loop {
            if let Ok(mut stream) = yaar_link::connect(&self.socket())
                && handshake(&mut stream, &mine).is_ok()
            {
                stream
                    .set_read_timeout(Some(DEADLINE))
                    .expect("set the deadline of a read");
                return Window { stream, next_id: 0 };
            }
            assert!(
                Instant::now() < deadline,
                "no Keeper at {:?}",
                self.socket()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// A repository in the folder `name` with one commit, `base.txt`, on `main`.
    fn repository(&self, name: &str) -> PathBuf {
        let folder = self.0.join(name);
        std::fs::create_dir_all(&folder).expect("make the folder");
        git(&folder, &["init", "--quiet", "-b", "main"]);
        // Git on Linux guesses no identity, and a repository's own is the one nothing of
        // the user's can change.
        git(&folder, &["config", "user.name", "A Test"]);
        git(&folder, &["config", "user.email", "test@example.invalid"]);
        write(&folder, "base.txt", "base\n");
        commit_all(&folder, "base");
        folder
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn git(folder: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(folder)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("start git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git printed text")
        .trim()
        .to_owned()
}

fn commit_all(folder: &Path, message: &str) -> String {
    git(folder, &["add", "--all"]);
    git(folder, &["commit", "--quiet", "-m", message]);
    git(folder, &["rev-parse", "HEAD"])
}

fn write(folder: &Path, name: &str, text: &str) {
    std::fs::write(folder.join(name), text).expect("write a file");
}

/// One end of the Link, as a Window holds it.
struct Window {
    stream: UnixStream,
    next_id: u32,
}

impl Window {
    /// Sends a request and expects its reply as the very next frame: nothing here leads
    /// to a fact.
    fn ask(&mut self, request: Request) -> Result<Reply, Refusal> {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        write_message(&mut self.stream, &Message::Request { id, request }).expect("send");
        match read_message(&mut self.stream).expect("a frame from the Keeper") {
            Message::Reply { to, reply } if to == id => reply,
            other => panic!("not the reply to {id:?}: {other:?}"),
        }
    }

    /// Sends a request and expects its reply, whatever facts come before it: a Keeper that
    /// watches the worktree of a running session tells the changes it sees to every Window.
    fn ask_past_facts(&mut self, request: Request) -> Result<Reply, Refusal> {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        write_message(&mut self.stream, &Message::Request { id, request }).expect("send");
        loop {
            match read_message(&mut self.stream).expect("a frame from the Keeper") {
                Message::Reply { to, reply } if to == id => return reply,
                Message::Fact(_) => {}
                other => panic!("not the reply to {id:?}: {other:?}"),
            }
        }
    }

    /// Reads frames until two sessions were told to have written `path` with their own
    /// tool, whatever else comes: the facts of a report are told after what the report did
    /// to the Keeper's records, and the reports were sent one after the other.
    fn until_written_by_two(&mut self, path: &str) {
        let mut writers: Vec<SessionId> = Vec::new();
        // The read timeout is for one frame: unrelated facts must not keep the wait going.
        let deadline = Instant::now() + DEADLINE;
        while writers.len() < 2 {
            assert!(
                Instant::now() < deadline,
                "two sessions were not told to have written {path} within {DEADLINE:?}"
            );
            if let Message::Fact(Fact::Step {
                session,
                step: Step::Write { path: written },
                ..
            }) = read_message(&mut self.stream).expect("a frame from the Keeper")
                && written == Path::new(path)
                && !writers.contains(&session)
            {
                writers.push(session);
            }
        }
    }

    /// Reads frames until `count` turns have ended with a state recorded, whatever else
    /// comes, and gives the sessions they were of with those states, in the order they were
    /// told. A turn's end is told after the state was recorded and its credits kept.
    fn until_turns_ended(&mut self, count: usize) -> Vec<(SessionId, String)> {
        let mut ended = Vec::new();
        // The read timeout is for one frame: unrelated facts must not keep the wait going.
        let deadline = Instant::now() + DEADLINE;
        while ended.len() < count {
            assert!(
                Instant::now() < deadline,
                "{count} turns did not end with a state within {DEADLINE:?}: {ended:?}"
            );
            if let Message::Fact(Fact::TurnEnded {
                session,
                state: Some(state),
                ..
            }) = read_message(&mut self.stream).expect("a frame from the Keeper")
            {
                ended.push((session, state));
            }
        }
        ended
    }

    fn stop(mut self, keeper: JoinHandle<std::io::Result<Exit>>) {
        write_message(&mut self.stream, &Message::Shutdown).expect("ask for the stop");
        assert_eq!(keeper.join().expect("join").expect("run"), Exit::Shutdown);
    }
}

/// What the Keeper keeps, made before the Keeper starts: the workspaces of the
/// repositories, the sessions, the turns, the states and the reviews, each with the call
/// the Keeper itself makes. Dropped before the Keeper starts, which then holds the
/// database alone.
struct Kept {
    store: Store,
    workspaces: Workspaces,
    reviews: Reviews,
}

impl Kept {
    fn new(scratch: &Scratch) -> Kept {
        std::fs::create_dir_all(scratch.data()).expect("make the app's folder");
        let store = Store::open(&scratch.data().join(yaar_store::FILE_NAME)).expect("the Store");
        let git = Git::find().expect("git");
        Kept {
            workspaces: Workspaces::new(git.clone(), store.clone()),
            reviews: Reviews::new(store.clone(), git, scratch.data().join("states")),
            store,
        }
    }

    /// The workspace of `repo`, and its worktree.
    fn open(&self, repo: &Path) -> (yaar_store::WorkspaceId, Arc<Worktree>) {
        let workspace = self.workspaces.open_folder(repo).expect("open").workspace;
        let worktree = self
            .workspaces
            .worktree(&workspace)
            .expect("the worktree")
            .expect("it is there");
        (workspace, worktree)
    }

    /// The state of the worktree as its files are now, by its full id.
    fn state(&self, workspace: &yaar_store::WorkspaceId, worktree: &Worktree) -> String {
        self.reviews
            .states_of(workspace)
            .expect("the state store")
            .record(worktree)
            .expect("record a state")
            .id
            .as_str()
            .to_owned()
    }

    /// A session of the claude Adapter that started, was given its review as the Keeper
    /// gives one, and ended, so that the next one that starts has a review of its own.
    fn session(&self, workspace: &yaar_store::WorkspaceId, n: u32) -> SessionRecord {
        let record = SessionRecord {
            id: yaar_store::SessionId::new(format!("session-{n}")),
            workspace: workspace.clone(),
            agent: "claude".to_owned(),
            cli_session: format!("cli-{n}"),
            terminal: Some(format!("terminal-{n}")),
            number: n,
            running: true,
            ended: None,
            ended_by: None,
        };
        self.store.put_session(&record).expect("keep the session");
        self.reviews.session_started(&record).expect("its review");
        self.store
            .set_ended(&record.id, EndedBy::Cli)
            .expect("end the session");
        record
    }

    /// The `number`th turn of `session`, as the Keeper keeps one that started.
    fn turn_begun(&self, session: &SessionRecord, number: u32) -> yaar_store::TurnId {
        let id = yaar_store::TurnId::new(format!("{}-turn-{number}", session.id));
        self.store
            .put_turn(&TurnRecord {
                id: id.clone(),
                session: session.id.clone(),
                number,
                ended: None,
                start: None,
                state: None,
            })
            .expect("keep the turn");
        id
    }

    /// A turn that ended `Finished` with `state`, which is none when its recording failed.
    fn turn(
        &self,
        session: &SessionRecord,
        number: u32,
        state: Option<&str>,
    ) -> yaar_store::TurnId {
        let id = self.turn_begun(session, number);
        self.store
            .end_turn(&id, TurnEndRecord::Finished, state)
            .expect("end the turn");
        id
    }
}

fn link_turn(id: &yaar_store::TurnId) -> TurnId {
    TurnId::new(id.as_str())
}

fn link_workspace(id: &yaar_store::WorkspaceId) -> WorkspaceId {
    WorkspaceId::new(id.as_str())
}

/// The history entry of a turn that ended.
fn turn_entry(
    id: &yaar_store::TurnId,
    session: &SessionRecord,
    state: Option<&str>,
) -> HistoryEntry {
    HistoryEntry::Turn {
        turn: link_turn(id),
        session: SessionId::new(session.id.as_str()),
        state: state.map(str::to_owned),
    }
}

fn commit_entry(id: &str, summary: &str) -> HistoryEntry {
    HistoryEntry::Commit(Commit {
        id: id.to_owned(),
        summary: summary.to_owned(),
    })
}

fn history_of(window: &mut Window, workspace: &yaar_store::WorkspaceId) -> History {
    match window.ask(Request::GetHistory {
        workspace: link_workspace(workspace),
    }) {
        Ok(Reply::History(history)) => history,
        other => panic!("not a history: {other:?}"),
    }
}

fn file(path: &str, status: FileStatus) -> ChangedFile {
    ChangedFile {
        path: PathBuf::from(path),
        status,
    }
}

fn credited(path: &str, status: FileStatus, by: &[&str]) -> CreditedFile {
    CreditedFile {
        file: file(path, status),
        by: by.iter().map(|session| SessionId::new(*session)).collect(),
    }
}

// The contract's first body for each request gives the refusal "does not answer yet": the
// assertions of these tests fail on it, as on any answer that is not the one they name.

#[test]
fn the_history_lists_commits_and_the_turns_that_ended_on_them_in_the_order_they_happened_and_asks_git_no_status()
 {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let base = git(&repo, &["rev-parse", "HEAD"]);
    git(&repo, &["switch", "--quiet", "-c", "feature"]);
    let kept = Kept::new(&scratch);
    let (workspace, worktree) = kept.open(&repo);
    let (a, b) = (kept.session(&workspace, 1), kept.session(&workspace, 2));

    write(&repo, "one.txt", "one\n");
    let c1 = commit_all(&repo, "first of the branch");
    // A state no turn made, as a comment on the files on disk leaves: not in the history.
    write(&repo, "note.txt", "a note\n");
    kept.state(&workspace, &worktree);
    // A's turn ends with its state; the next commit takes the file in.
    write(&repo, "two.txt", "two\n");
    let s1 = kept.state(&workspace, &worktree);
    let t1 = kept.turn(&a, 1, Some(&s1));
    let c2 = commit_all(&repo, "second of the branch");
    write(&repo, "three.txt", "three\n");
    let s2 = kept.state(&workspace, &worktree);
    let t2 = kept.turn(&b, 1, Some(&s2));
    write(&repo, "four.txt", "four\n");
    kept.state(&workspace, &worktree);
    drop((kept, worktree));
    // The Keeper's own `git status` of this worktree would now fail: the list of its
    // files is no list, and a lock lies on it.
    std::fs::write(repo.join(".git/index"), "not a list of files").expect("break the index");
    std::fs::write(repo.join(".git/index.lock"), "").expect("lock the index");
    let keeper = scratch.keeper();
    let mut window = scratch.window();

    assert!(
        matches!(
            window.ask(Request::GetChanges {
                workspace: link_workspace(&workspace),
            }),
            Err(Refusal::Failed { .. })
        ),
        "the control: reading the changes of this worktree needs git status, which fails here"
    );
    let history = history_of(&mut window, &workspace);

    assert_eq!(history.workspace, link_workspace(&workspace));
    assert_eq!(
        history.start,
        Some(base),
        "counted from where the branch left main"
    );
    assert_eq!(
        history.entries,
        [
            commit_entry(&c1, "first of the branch"),
            turn_entry(&t1, &a, Some(&s1)),
            commit_entry(&c2, "second of the branch"),
            turn_entry(&t2, &b, Some(&s2)),
        ]
    );
    window.stop(keeper);
}

#[test]
fn a_turn_whose_state_is_none_or_no_longer_kept_is_listed_with_no_state_after_the_turn_before_it() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    git(&repo, &["switch", "--quiet", "-c", "feature"]);
    let kept = Kept::new(&scratch);
    let (workspace, worktree) = kept.open(&repo);
    let a = kept.session(&workspace, 1);

    write(&repo, "one.txt", "one\n");
    let s1 = kept.state(&workspace, &worktree);
    let t1 = kept.turn(&a, 1, Some(&s1));
    // The recording failed.
    let t2 = kept.turn(&a, 2, None);
    // A state the store never held, as one it has lost.
    let lost = "0".repeat(40);
    let t3 = kept.turn(&a, 3, Some(&lost));
    let c1 = commit_all(&repo, "first of the branch");
    write(&repo, "two.txt", "two\n");
    let s4 = kept.state(&workspace, &worktree);
    let t4 = kept.turn(&a, 4, Some(&s4));
    drop((kept, worktree));
    let keeper = scratch.keeper();
    let mut window = scratch.window();

    let history = history_of(&mut window, &workspace);

    assert_eq!(
        history.entries,
        [
            turn_entry(&t1, &a, Some(&s1)),
            turn_entry(&t2, &a, None),
            turn_entry(&t3, &a, None),
            commit_entry(&c1, "first of the branch"),
            turn_entry(&t4, &a, Some(&s4)),
        ]
    );
    window.stop(keeper);
}

#[test]
fn turns_with_no_state_and_no_turn_with_one_are_listed_after_the_last_commit_in_store_order() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    git(&repo, &["switch", "--quiet", "-c", "feature"]);
    let kept = Kept::new(&scratch);
    let (workspace, worktree) = kept.open(&repo);
    let a = kept.session(&workspace, 1);

    // Two commits of the branch, then only turns with no state: nothing places them among
    // the commits, and a turn is not listed before a commit it may well have followed.
    write(&repo, "one.txt", "one\n");
    let c1 = commit_all(&repo, "first of the branch");
    write(&repo, "two.txt", "two\n");
    let c2 = commit_all(&repo, "second of the branch");
    let t1 = kept.turn(&a, 1, None);
    // A state the store never held, as one it has lost.
    let lost = "0".repeat(40);
    let t2 = kept.turn(&a, 2, Some(&lost));
    let t3 = kept.turn(&a, 3, None);
    drop((kept, worktree));
    let keeper = scratch.keeper();
    let mut window = scratch.window();

    let history = history_of(&mut window, &workspace);

    assert_eq!(
        history.entries,
        [
            commit_entry(&c1, "first of the branch"),
            commit_entry(&c2, "second of the branch"),
            turn_entry(&t1, &a, None),
            turn_entry(&t2, &a, None),
            turn_entry(&t3, &a, None),
        ],
        "after the last commit, not before the first"
    );
    window.stop(keeper);
}

#[test]
fn a_turn_that_runs_one_of_another_workspace_and_one_before_the_start_are_not_listed() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let other = scratch.repository("other");
    let kept = Kept::new(&scratch);
    let (workspace, worktree) = kept.open(&repo);
    let (elsewhere, other_worktree) = kept.open(&other);
    let (a, b) = (kept.session(&workspace, 1), kept.session(&elsewhere, 2));

    // On main with no remote the changes count from the commit checked out: a turn that
    // ended before the next commit is before the start of what Changes shows.
    write(&repo, "one.txt", "one\n");
    let before = kept.state(&workspace, &worktree);
    kept.turn(&a, 1, Some(&before));
    git(&repo, &["switch", "--quiet", "-c", "feature"]);
    commit_all(&repo, "first of the branch");
    git(&repo, &["switch", "--quiet", "main"]);
    git(&repo, &["merge", "--quiet", "--ff-only", "feature"]);
    // Another workspace's turn, with its own state.
    write(&other, "two.txt", "two\n");
    let theirs = kept.state(&elsewhere, &other_worktree);
    kept.turn(&b, 1, Some(&theirs));
    // A turn of this session that has not ended.
    kept.turn_begun(&a, 2);
    // One that ended on the commit now checked out is in.
    write(&repo, "three.txt", "three\n");
    let now = kept.state(&workspace, &worktree);
    let t3 = kept.turn(&a, 3, Some(&now));
    drop((kept, worktree, other_worktree));
    let keeper = scratch.keeper();
    let mut window = scratch.window();

    let history = history_of(&mut window, &workspace);

    assert_eq!(history.entries, [turn_entry(&t3, &a, Some(&now))]);
    window.stop(keeper);
}

#[test]
fn a_turn_with_no_state_and_no_turn_before_it_is_listed_before_the_next_turn_with_a_state() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    git(&repo, &["switch", "--quiet", "-c", "feature"]);
    let kept = Kept::new(&scratch);
    let (workspace, worktree) = kept.open(&repo);
    let a = kept.session(&workspace, 1);

    // The first turn of the workspace has no state; a commit follows, then a turn with one.
    // Nothing the Store holds places the first turn among the commits (it really ended
    // before the commit), so just before the next turn with a state is a choice made for
    // lack of a fact, and the order asserted is that choice, not what happened.
    let t1 = kept.turn(&a, 1, None);
    write(&repo, "one.txt", "one\n");
    let c1 = commit_all(&repo, "first of the branch");
    write(&repo, "two.txt", "two\n");
    let s2 = kept.state(&workspace, &worktree);
    let t2 = kept.turn(&a, 2, Some(&s2));
    drop((kept, worktree));
    let keeper = scratch.keeper();
    let mut window = scratch.window();

    let history = history_of(&mut window, &workspace);

    assert_eq!(
        history.entries,
        [
            commit_entry(&c1, "first of the branch"),
            turn_entry(&t1, &a, None),
            turn_entry(&t2, &a, Some(&s2)),
        ],
        "not at the end, after every commit"
    );
    window.stop(keeper);
}

#[test]
fn a_turn_with_no_state_that_follows_one_before_the_start_is_listed() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let kept = Kept::new(&scratch);
    let (workspace, worktree) = kept.open(&repo);
    let a = kept.session(&workspace, 1);

    // On main with no remote the changes count from the commit checked out: the first turn
    // ended before the next commit, so it is before the start of what Changes shows.
    write(&repo, "one.txt", "one\n");
    let before = kept.state(&workspace, &worktree);
    kept.turn(&a, 1, Some(&before));
    git(&repo, &["switch", "--quiet", "-c", "feature"]);
    commit_all(&repo, "first of the branch");
    git(&repo, &["switch", "--quiet", "main"]);
    git(&repo, &["merge", "--quiet", "--ff-only", "feature"]);
    // The next turn ended after the start; its recording failed.
    let t2 = kept.turn(&a, 2, None);
    write(&repo, "two.txt", "two\n");
    let now = kept.state(&workspace, &worktree);
    let t3 = kept.turn(&a, 3, Some(&now));
    drop((kept, worktree));
    let keeper = scratch.keeper();
    let mut window = scratch.window();

    let history = history_of(&mut window, &workspace);

    assert_eq!(
        history.entries,
        [turn_entry(&t2, &a, None), turn_entry(&t3, &a, Some(&now))]
    );
    window.stop(keeper);
}

#[test]
fn a_turn_with_no_state_that_started_before_one_before_the_start_is_not_listed() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let kept = Kept::new(&scratch);
    let (workspace, worktree) = kept.open(&repo);
    let a = kept.session(&workspace, 1);

    // The first turn has no state, and the turn after it ended before the start of what
    // Changes shows: the first started before that one, so it ended before the start too.
    kept.turn(&a, 1, None);
    write(&repo, "one.txt", "one\n");
    let before = kept.state(&workspace, &worktree);
    kept.turn(&a, 2, Some(&before));
    git(&repo, &["switch", "--quiet", "-c", "feature"]);
    commit_all(&repo, "first of the branch");
    git(&repo, &["switch", "--quiet", "main"]);
    git(&repo, &["merge", "--quiet", "--ff-only", "feature"]);
    write(&repo, "two.txt", "two\n");
    let now = kept.state(&workspace, &worktree);
    let t3 = kept.turn(&a, 3, Some(&now));
    drop((kept, worktree));
    let keeper = scratch.keeper();
    let mut window = scratch.window();

    let history = history_of(&mut window, &workspace);

    assert_eq!(history.entries, [turn_entry(&t3, &a, Some(&now))]);
    window.stop(keeper);
}

#[test]
fn a_workspace_with_nothing_committed_has_no_start_and_lists_its_turns() {
    let scratch = Scratch::new();
    let repo = scratch.0.join("empty");
    std::fs::create_dir_all(&repo).expect("make the folder");
    git(&repo, &["init", "--quiet", "-b", "main"]);
    let kept = Kept::new(&scratch);
    let (workspace, worktree) = kept.open(&repo);
    let a = kept.session(&workspace, 1);
    write(&repo, "one.txt", "one\n");
    let s1 = kept.state(&workspace, &worktree);
    let t1 = kept.turn(&a, 1, Some(&s1));
    drop((kept, worktree));
    let keeper = scratch.keeper();
    let mut window = scratch.window();

    let history = history_of(&mut window, &workspace);

    assert_eq!(history.start, None);
    assert_eq!(history.entries, [turn_entry(&t1, &a, Some(&s1))]);
    window.stop(keeper);
}

#[test]
fn the_history_of_a_workspace_the_keeper_does_not_hold_or_whose_worktree_is_gone_is_refused() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let kept = Kept::new(&scratch);
    let (workspace, _) = kept.open(&repo);
    drop(kept);
    let keeper = scratch.keeper();
    let mut window = scratch.window();

    let unknown = WorkspaceId::new("no-such-workspace");
    assert_eq!(
        window.ask(Request::GetHistory {
            workspace: unknown.clone()
        }),
        Err(Refusal::UnknownWorkspace { workspace: unknown })
    );
    let moved = scratch.0.join("moved");
    std::fs::rename(&repo, &moved).expect("move the worktree away");
    assert_eq!(
        window.ask(Request::GetHistory {
            workspace: link_workspace(&workspace)
        }),
        Err(Refusal::UnknownWorkspace {
            workspace: link_workspace(&workspace)
        })
    );
    window.stop(keeper);
}

#[test]
fn a_turn_is_read_with_its_steps_in_order_also_while_it_runs_and_an_unknown_one_is_refused() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let kept = Kept::new(&scratch);
    let (workspace, worktree) = kept.open(&repo);
    let a = kept.session(&workspace, 1);
    write(&repo, "one.txt", "one\n");
    let s1 = kept.state(&workspace, &worktree);
    let ended = kept.turn(&a, 1, Some(&s1));
    for (number, step) in [
        StepRecord::Read(PathBuf::from("base.txt")),
        StepRecord::Edit(PathBuf::from("base.txt"), Some(vec![2..=4, 9..=9])),
        StepRecord::Edit(PathBuf::from("other.txt"), None),
        StepRecord::Write(PathBuf::from("one.txt")),
        StepRecord::Command("cargo test".to_owned()),
    ]
    .into_iter()
    .enumerate()
    {
        let number = u32::try_from(number).expect("a few steps") + 1;
        kept.store
            .put_step(&ended, number, &step)
            .expect("keep the step");
    }
    let running = kept.turn_begun(&a, 2);
    kept.store
        .put_step(&running, 1, &StepRecord::Command("ls".to_owned()))
        .expect("keep the step");
    drop((kept, worktree));
    let keeper = scratch.keeper();
    let mut window = scratch.window();

    assert_eq!(
        window.ask(Request::GetTurn {
            turn: link_turn(&ended)
        }),
        Ok(Reply::Turn(Turn {
            id: link_turn(&ended),
            session: SessionId::new(a.id.as_str()),
            number: 1,
            ended: Some(TurnEnd::Finished),
            start: None,
            state: Some(s1),
            steps: vec![
                Step::Read {
                    path: PathBuf::from("base.txt")
                },
                Step::Edit {
                    path: PathBuf::from("base.txt"),
                    lines: Some(vec![2..=4, 9..=9]),
                },
                Step::Edit {
                    path: PathBuf::from("other.txt"),
                    lines: None,
                },
                Step::Write {
                    path: PathBuf::from("one.txt")
                },
                Step::Command {
                    command: "cargo test".to_owned()
                },
            ],
        }))
    );
    assert_eq!(
        window.ask(Request::GetTurn {
            turn: link_turn(&running)
        }),
        Ok(Reply::Turn(Turn {
            id: link_turn(&running),
            session: SessionId::new(a.id.as_str()),
            number: 2,
            ended: None,
            start: None,
            state: None,
            steps: vec![Step::Command {
                command: "ls".to_owned()
            }],
        }))
    );
    let unknown = TurnId::new("no-such-turn");
    assert_eq!(
        window.ask(Request::GetTurn {
            turn: unknown.clone()
        }),
        Err(Refusal::UnknownTurn { turn: unknown })
    );
    window.stop(keeper);
}

/// A repository with `a.txt`, `gone.txt` and `moved.txt` committed on `main`, one commit
/// more on `feature` that renames, edits and removes, and a state of the files on top.
struct Versions {
    repo: PathBuf,
    workspace: yaar_store::WorkspaceId,
    c1: String,
    c2: String,
    state: String,
}

fn versions(scratch: &Scratch, kept: &Kept) -> Versions {
    let repo = scratch.repository("repo");
    // Several lines each, so that git pairs a rename by what the file holds.
    let moved = "the file that is moved\nfrom one place\nto another\nand keeps its lines\n";
    write(&repo, "a.txt", "one\ntwo\nthree\n");
    write(&repo, "gone.txt", "to be removed\n");
    write(&repo, "moved.txt", moved);
    let c1 = commit_all(&repo, "files");
    git(&repo, &["switch", "--quiet", "-c", "feature"]);
    write(&repo, "a.txt", "one\n2\nthree\n");
    git(&repo, &["rm", "--quiet", "gone.txt"]);
    git(&repo, &["mv", "moved.txt", "renamed.txt"]);
    write(&repo, "new.txt", "new\n");
    let c2 = commit_all(&repo, "changes");
    write(&repo, "a.txt", "one\n2\nthree\nfour\n");
    let (workspace, worktree) = kept.open(&repo);
    let state = kept.state(&workspace, &worktree);
    Versions {
        repo,
        workspace,
        c1,
        c2,
        state,
    }
}

fn compare(
    window: &mut Window,
    workspace: &yaar_store::WorkspaceId,
    from: &str,
    to: Option<&str>,
) -> Result<Reply, Refusal> {
    window.ask(Request::CompareVersions {
        workspace: link_workspace(workspace),
        from: from.to_owned(),
        to: to.map(str::to_owned),
    })
}

#[test]
fn two_versions_are_compared_whether_commits_states_or_the_files_on_disk() {
    let scratch = Scratch::new();
    let kept = Kept::new(&scratch);
    let v = versions(&scratch, &kept);
    drop(kept);
    let keeper = scratch.keeper();
    let mut window = scratch.window();

    // Two commits, ordered by path, a rename paired with its old path.
    assert_eq!(
        compare(&mut window, &v.workspace, &v.c1, Some(&v.c2)),
        Ok(Reply::Compared(vec![
            file("a.txt", FileStatus::Modified),
            file("gone.txt", FileStatus::Deleted),
            file("new.txt", FileStatus::Added),
            file(
                "renamed.txt",
                FileStatus::Renamed {
                    from: PathBuf::from("moved.txt")
                }
            ),
        ]))
    );
    // A commit and a state taken on the next one.
    assert_eq!(
        compare(&mut window, &v.workspace, &v.c2, Some(&v.state)),
        Ok(Reply::Compared(vec![file("a.txt", FileStatus::Modified)]))
    );
    // A version and the files on disk, which hold one more file than the state.
    write(&v.repo, "later.txt", "later\n");
    assert_eq!(
        compare(&mut window, &v.workspace, &v.state, None),
        Ok(Reply::Compared(vec![file("later.txt", FileStatus::Added)]))
    );
    assert_eq!(
        compare(&mut window, &v.workspace, &v.c2, None),
        Ok(Reply::Compared(vec![
            file("a.txt", FileStatus::Modified),
            file("later.txt", FileStatus::Added),
        ]))
    );
    // The same version on both sides differs in nothing.
    assert_eq!(
        compare(&mut window, &v.workspace, &v.c1, Some(&v.c1)),
        Ok(Reply::Compared(Vec::new()))
    );
    window.stop(keeper);
}

#[test]
fn a_comparison_names_the_version_the_keeper_does_not_know_and_the_workspace_it_does_not_hold() {
    let scratch = Scratch::new();
    let kept = Kept::new(&scratch);
    let v = versions(&scratch, &kept);
    drop(kept);
    let keeper = scratch.keeper();
    let mut window = scratch.window();

    let missing = "0".repeat(40);
    for unknown in ["HEAD", "main", "not-an-id", &missing] {
        let refused = Err(Refusal::UnknownRevision {
            revision: unknown.to_owned(),
        });
        assert_eq!(
            compare(&mut window, &v.workspace, unknown, Some(&v.c1)),
            refused,
            "{unknown} as the first"
        );
        assert_eq!(
            compare(&mut window, &v.workspace, &v.c1, Some(unknown)),
            refused,
            "{unknown} as the second"
        );
    }
    let nobody = yaar_store::WorkspaceId::new("no-such-workspace");
    assert_eq!(
        compare(&mut window, &nobody, &v.c1, Some(&v.c2)),
        Err(Refusal::UnknownWorkspace {
            workspace: link_workspace(&nobody)
        })
    );
    window.stop(keeper);
}

#[test]
fn what_the_store_holds_is_compared_when_the_worktree_is_gone_and_the_files_on_disk_are_not() {
    let scratch = Scratch::new();
    let kept = Kept::new(&scratch);
    let v = versions(&scratch, &kept);
    drop(kept);
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    // The commit of the state, kept in the store while the worktree is there.
    assert!(matches!(
        compare(&mut window, &v.workspace, &v.c2, Some(&v.state)),
        Ok(Reply::Compared(_))
    ));
    std::fs::rename(&v.repo, scratch.0.join("moved")).expect("move the worktree away");

    assert_eq!(
        compare(&mut window, &v.workspace, &v.c2, Some(&v.state)),
        Ok(Reply::Compared(vec![file("a.txt", FileStatus::Modified)]))
    );
    assert!(
        matches!(
            compare(&mut window, &v.workspace, &v.c2, None),
            Err(Refusal::Failed { .. })
        ),
        "there are no files on disk to compare with"
    );
    window.stop(keeper);
}

fn read(
    window: &mut Window,
    workspace: &yaar_store::WorkspaceId,
    old: Option<(&str, &str)>,
    new: (Option<&str>, &str),
) -> Result<Reply, Refusal> {
    window.ask(Request::ReadVersions {
        workspace: link_workspace(workspace),
        old: old.map(|(version, _)| version.to_owned()),
        old_path: PathBuf::from(old.map_or("", |(_, path)| path)),
        new: new.0.map(str::to_owned),
        path: PathBuf::from(new.1),
    })
}

fn sides(old: Content, new: Content) -> Result<Reply, Refusal> {
    Ok(Reply::File(FileSides { old, new }))
}

fn text(text: &str) -> Content {
    Content::Text(text.as_bytes().to_vec())
}

#[test]
fn the_two_sides_of_a_file_are_read_in_any_two_versions_and_on_disk_each_at_its_own_path() {
    let scratch = Scratch::new();
    let kept = Kept::new(&scratch);
    let v = versions(&scratch, &kept);
    drop(kept);
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let ws = &v.workspace;

    // A commit and a state.
    assert_eq!(
        read(
            &mut window,
            ws,
            Some((&v.c1, "a.txt")),
            (Some(&v.state), "a.txt")
        ),
        sides(text("one\ntwo\nthree\n"), text("one\n2\nthree\nfour\n"))
    );
    // The file on disk, which is not what the state holds.
    write(&v.repo, "a.txt", "on disk\n");
    assert_eq!(
        read(&mut window, ws, Some((&v.c2, "a.txt")), (None, "a.txt")),
        sides(text("one\n2\nthree\n"), text("on disk\n"))
    );
    // A renamed file: each side at the path it had.
    assert_eq!(
        read(
            &mut window,
            ws,
            Some((&v.c1, "moved.txt")),
            (Some(&v.c2), "renamed.txt")
        ),
        sides(
            text("the file that is moved\nfrom one place\nto another\nand keeps its lines\n"),
            text("the file that is moved\nfrom one place\nto another\nand keeps its lines\n")
        )
    );
    // A file that is not there on a side, and no old side asked for.
    assert_eq!(
        read(
            &mut window,
            ws,
            Some((&v.c1, "new.txt")),
            (Some(&v.c2), "new.txt")
        ),
        sides(Content::Absent, text("new\n"))
    );
    assert_eq!(
        read(
            &mut window,
            ws,
            Some((&v.c1, "gone.txt")),
            (Some(&v.c2), "gone.txt")
        ),
        sides(text("to be removed\n"), Content::Absent)
    );
    assert_eq!(
        read(&mut window, ws, None, (None, "no-such-file.txt")),
        sides(Content::Absent, Content::Absent)
    );
    assert_eq!(
        read(&mut window, ws, None, (Some(&v.c2), "new.txt")),
        sides(Content::Absent, text("new\n"))
    );
    window.stop(keeper);
}

#[test]
fn a_side_that_is_not_text_or_too_large_to_travel_is_sent_as_its_size_alone() {
    let scratch = Scratch::new();
    let kept = Kept::new(&scratch);
    let v = versions(&scratch, &kept);
    drop(kept);
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let too_large = usize::try_from(MAX_CONTENT).expect("a size") + 1;
    std::fs::write(v.repo.join("binary.bin"), b"abc\0def").expect("write");
    std::fs::write(v.repo.join("large.txt"), vec![b'x'; too_large]).expect("write");
    // Past the first 8,000 bytes a zero byte does not make a file binary, as git has it.
    let mut late = vec![b'y'; 8000];
    late.push(0);
    std::fs::write(v.repo.join("late.bin"), &late).expect("write");
    // A file that is both too large and not text is said to be not text.
    let mut huge = vec![0u8];
    huge.extend(vec![b'x'; too_large]);
    std::fs::write(v.repo.join("huge.bin"), &huge).expect("write");

    assert_eq!(
        read(&mut window, &v.workspace, None, (None, "binary.bin")),
        sides(Content::Absent, Content::Binary { size: 7 })
    );
    assert_eq!(
        read(&mut window, &v.workspace, None, (None, "large.txt")),
        sides(
            Content::Absent,
            Content::TooLarge {
                size: too_large as u64
            }
        )
    );
    assert_eq!(
        read(&mut window, &v.workspace, None, (None, "huge.bin")),
        sides(
            Content::Absent,
            Content::Binary {
                size: huge.len() as u64
            }
        )
    );
    assert_eq!(
        read(&mut window, &v.workspace, None, (None, "late.bin")),
        sides(Content::Absent, Content::Text(late))
    );
    window.stop(keeper);
}

#[test]
fn reading_the_sides_of_a_file_refuses_what_leads_out_of_the_worktree_and_what_is_unknown() {
    let scratch = Scratch::new();
    let kept = Kept::new(&scratch);
    let v = versions(&scratch, &kept);
    drop(kept);
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let ws = &v.workspace;

    for outside in ["../outside", "/etc/hostname", ""] {
        assert_eq!(
            read(&mut window, ws, None, (None, outside)),
            Err(Refusal::OutsideWorkspace {
                path: PathBuf::from(outside)
            }),
            "{outside:?} as the new path"
        );
        assert_eq!(
            read(&mut window, ws, Some((&v.c1, outside)), (None, "a.txt")),
            Err(Refusal::OutsideWorkspace {
                path: PathBuf::from(outside)
            }),
            "{outside:?} as the old path"
        );
    }
    let missing = "0".repeat(40);
    for unknown in ["HEAD", &missing] {
        let refused = Err(Refusal::UnknownRevision {
            revision: unknown.to_owned(),
        });
        assert_eq!(
            read(&mut window, ws, Some((unknown, "a.txt")), (None, "a.txt")),
            refused,
            "{unknown} as the old version"
        );
        assert_eq!(
            read(&mut window, ws, None, (Some(unknown), "a.txt")),
            refused,
            "{unknown} as the new version"
        );
    }
    let nobody = yaar_store::WorkspaceId::new("no-such-workspace");
    assert_eq!(
        read(&mut window, &nobody, None, (None, "a.txt")),
        Err(Refusal::UnknownWorkspace {
            workspace: link_workspace(&nobody)
        })
    );
    window.stop(keeper);
}

/// A workspace with two sessions that each have a review, and a worktree with three
/// uncommitted changes: a modified file, a new one and a removed one.
struct Reviewed {
    repo: PathBuf,
    workspace: yaar_store::WorkspaceId,
    review_a: ReviewId,
    review_b: ReviewId,
}

fn reviewed(scratch: &Scratch, kept: &Kept) -> Reviewed {
    let repo = scratch.repository("repo");
    write(&repo, "kept.txt", "kept\n");
    write(&repo, "removed.txt", "removed\n");
    commit_all(&repo, "files");
    let (workspace, _) = kept.open(&repo);
    let [a, b] = [1, 2].map(|n| {
        let record = SessionRecord {
            id: yaar_store::SessionId::new(format!("session-{n}")),
            workspace: workspace.clone(),
            agent: "claude".to_owned(),
            cli_session: format!("cli-{n}"),
            terminal: Some(format!("terminal-{n}")),
            number: n,
            running: true,
            ended: None,
            ended_by: None,
        };
        kept.store.put_session(&record).expect("keep the session");
        // Both run at once, as the review of one is never taken by the other.
        let review = kept.reviews.session_started(&record).expect("its review");
        ReviewId::new(review.as_str())
    });
    write(&repo, "kept.txt", "changed\n");
    write(&repo, "added.txt", "added\n");
    std::fs::remove_file(repo.join("removed.txt")).expect("remove a file");
    Reviewed {
        repo,
        workspace,
        review_a: a,
        review_b: b,
    }
}

fn uncommitted(
    window: &mut Window,
    workspace: &yaar_store::WorkspaceId,
    review: Option<&ReviewId>,
) -> Result<Reply, Refusal> {
    window.ask(Request::GetUncommitted {
        workspace: link_workspace(workspace),
        review: review.cloned(),
    })
}

/// [`uncommitted`] for a Keeper that tells facts of its own beside the reply.
fn uncommitted_past_facts(
    window: &mut Window,
    workspace: &yaar_store::WorkspaceId,
    review: Option<&ReviewId>,
) -> Result<Reply, Refusal> {
    window.ask_past_facts(Request::GetUncommitted {
        workspace: link_workspace(workspace),
        review: review.cloned(),
    })
}

#[test]
fn with_no_agent_credited_with_a_change_every_review_lists_all_of_them_by_path_with_nobody_as_its_maker()
 {
    let scratch = Scratch::new();
    let kept = Kept::new(&scratch);
    let r = reviewed(&scratch, &kept);
    // The review of no agent of another workspace, in which nothing is credited either.
    let other = scratch.repository("other");
    let (elsewhere, _) = kept.open(&other);
    write(&other, "x.txt", "x\n");
    let none = kept
        .reviews
        .of_workspace(&elsewhere, None)
        .expect("its review")
        .reviews[0]
        .id()
        .clone();
    drop(kept);
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let every = Ok(Reply::Uncommitted(vec![
        credited("added.txt", FileStatus::Added, &[]),
        credited("kept.txt", FileStatus::Modified, &[]),
        credited("removed.txt", FileStatus::Deleted, &[]),
    ]));

    // No review named: every change.
    assert_eq!(uncommitted(&mut window, &r.workspace, None), every);
    // A review of a session that made none: the changes no agent made, which are all.
    assert_eq!(
        uncommitted(&mut window, &r.workspace, Some(&r.review_a)),
        every
    );
    assert_eq!(
        uncommitted(&mut window, &r.workspace, Some(&r.review_b)),
        every
    );
    // The review of no agent.
    assert_eq!(
        uncommitted(&mut window, &elsewhere, Some(&ReviewId::new(none.as_str()))),
        Ok(Reply::Uncommitted(vec![credited(
            "x.txt",
            FileStatus::Added,
            &[]
        )]))
    );
    // Committed, a change is listed no more.
    commit_all(&r.repo, "everything");
    assert_eq!(
        uncommitted(&mut window, &r.workspace, Some(&r.review_a)),
        Ok(Reply::Uncommitted(Vec::new()))
    );
    window.stop(keeper);
}

#[test]
fn the_uncommitted_changes_of_a_review_the_keeper_does_not_hold_or_of_another_workspace_are_refused()
 {
    let scratch = Scratch::new();
    let kept = Kept::new(&scratch);
    let r = reviewed(&scratch, &kept);
    let other = scratch.repository("other");
    let (elsewhere, _) = kept.open(&other);
    drop(kept);
    let keeper = scratch.keeper();
    let mut window = scratch.window();

    let unknown = ReviewId::new("no-such-review");
    assert_eq!(
        uncommitted(&mut window, &r.workspace, Some(&unknown)),
        Err(Refusal::UnknownReview { review: unknown })
    );
    // A review of the first workspace names no change of the second.
    assert_eq!(
        uncommitted(&mut window, &elsewhere, Some(&r.review_a)),
        Err(Refusal::UnknownReview {
            review: r.review_a.clone()
        })
    );
    let nobody = yaar_store::WorkspaceId::new("no-such-workspace");
    assert_eq!(
        uncommitted(&mut window, &nobody, Some(&r.review_a)),
        Err(Refusal::UnknownWorkspace {
            workspace: link_workspace(&nobody)
        })
    );
    assert_eq!(
        uncommitted(&mut window, &nobody, None),
        Err(Refusal::UnknownWorkspace {
            workspace: link_workspace(&nobody)
        })
    );
    window.stop(keeper);
}

/// The hooks of the recording of two sessions in one folder that tell a session started,
/// its first prompt, and one write of a file with its own tool, each as the CLI's piece
/// would pass them on, for the files `paths`, read as if the recording was made in `repo`.
fn writes_of(session: usize, repo: &Path, paths: &[&str]) -> Vec<Vec<u8>> {
    let events = yaar_system_tests::recorded::hook_events("claude/two-sessions-files.jsonl");
    let mut sessions: Vec<&str> = Vec::new();
    for event in events.iter().filter(|event| event.event == "SessionStart") {
        if !sessions.contains(&event.session()) {
            sessions.push(event.session());
        }
    }
    let mine = sessions[session];
    let of_mine = |name: &str, tool: Option<&str>| {
        events
            .iter()
            .find(|event| {
                event.session() == mine
                    && event.event == name
                    && tool.is_none_or(|tool| event.input["tool_name"] == tool)
            })
            .unwrap_or_else(|| panic!("no {name} of session {session} in the recording"))
            .in_project(repo)
    };
    let mut payloads = vec![
        of_mine("SessionStart", None).input_bytes(),
        of_mine("UserPromptSubmit", None).input_bytes(),
    ];
    for path in paths {
        for event in ["PreToolUse", "PostToolUse"] {
            let mut write = of_mine(event, Some("Write"));
            let file = repo.join(path);
            write.input["tool_input"]["file_path"] = file.to_string_lossy().into_owned().into();
            if let Some(response) = write.input.get_mut("tool_response")
                && response.get("filePath").is_some()
            {
                response["filePath"] = file.to_string_lossy().into_owned().into();
            }
            payloads.push(write.input_bytes());
        }
    }
    payloads
}

#[test]
fn a_session_lists_the_changes_it_made_and_those_no_agent_made_and_one_made_by_two_is_in_both() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let kept = Kept::new(&scratch);
    let (workspace, _) = kept.open(&repo);
    drop(kept);
    for (name, text) in [
        ("a-only.txt", "a\n"),
        ("b-only.txt", "b\n"),
        ("both.txt", "both\n"),
        ("nobody.txt", "nobody\n"),
    ] {
        write(&repo, name, text);
    }
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    // Each session runs in a shell of the app of its own. The Windows that open them are
    // kept to the end: the output of a shell goes to the one that opened it.
    let size = TerminalSize {
        columns: 80,
        rows: 24,
    };
    let mut openers = Vec::new();
    let mut terminals: Vec<TerminalId> = Vec::new();
    for _ in 0..2 {
        let mut opener = scratch.window();
        let Ok(Reply::TerminalOpened { terminal }) = opener.ask(Request::OpenTerminal {
            workspace: link_workspace(&workspace),
            program: Program::Shell,
            size,
        }) else {
            panic!("no terminal");
        };
        terminals.push(terminal);
        openers.push(opener);
    }
    // Two processes that are alive for as long as the test runs: this one and its parent.
    // SAFETY: `getppid` takes no argument and cannot fail.
    let pids = [
        std::process::id(),
        unsafe { libc::getppid() }.unsigned_abs(),
    ];

    let reports = [
        writes_of(0, &repo, &["a-only.txt", "both.txt"]),
        writes_of(1, &repo, &["b-only.txt", "both.txt"]),
    ];
    // One connection for each report: a piece of a CLI leaves once it is answered, and
    // reads past the facts told before its answer.
    for (session, payloads) in reports.into_iter().enumerate() {
        for payload in payloads {
            let answered = scratch.window().ask_past_facts(Request::Report {
                report: AgentReport {
                    agent: "claude".to_owned(),
                    terminal: Some(terminals[session].clone()),
                    pid: pids[session],
                    payload,
                },
            });
            assert_eq!(answered, Ok(Reply::Done));
        }
    }
    // The barrier: the last file the reports name was written by both, and a step is told
    // after the Keeper noted who wrote the file.
    window.until_written_by_two("both.txt");
    let Ok(Reply::Agents(agents)) = window.ask_past_facts(Request::ListAgents) else {
        panic!("no list of agents");
    };
    let sessions = &agents
        .iter()
        .find(|listed| listed.workspace == link_workspace(&workspace))
        .expect("the workspace has sessions")
        .sessions;
    let [a, b] = [0, 1].map(|n| &sessions[n]);
    let (review_a, review_b) = (
        a.review.clone().expect("A has a review"),
        b.review.clone().expect("B has a review"),
    );
    assert_ne!(review_a, review_b);
    let (a, b) = (a.id.as_str(), b.id.as_str());

    assert_eq!(
        uncommitted_past_facts(&mut window, &workspace, Some(&review_a)),
        Ok(Reply::Uncommitted(vec![
            credited("a-only.txt", FileStatus::Added, &[a]),
            credited("both.txt", FileStatus::Added, &[a, b]),
            credited("nobody.txt", FileStatus::Added, &[]),
        ]))
    );
    assert_eq!(
        uncommitted_past_facts(&mut window, &workspace, Some(&review_b)),
        Ok(Reply::Uncommitted(vec![
            credited("b-only.txt", FileStatus::Added, &[b]),
            credited("both.txt", FileStatus::Added, &[a, b]),
            credited("nobody.txt", FileStatus::Added, &[]),
        ]))
    );
    assert_eq!(
        uncommitted_past_facts(&mut window, &workspace, None),
        Ok(Reply::Uncommitted(vec![
            credited("a-only.txt", FileStatus::Added, &[a]),
            credited("b-only.txt", FileStatus::Added, &[b]),
            credited("both.txt", FileStatus::Added, &[a, b]),
            credited("nobody.txt", FileStatus::Added, &[]),
        ]))
    );
    window.stop(keeper);
}

/// Two sessions that run in one workspace, each with the review the Keeper gives it, and two
/// states of its worktree between which they are compared: the first of a clean worktree,
/// the second of a worktree with every kind of change in it, kept with who made each.
struct Compared {
    workspace: yaar_store::WorkspaceId,
    review_a: ReviewId,
    review_b: ReviewId,
    review_elsewhere: ReviewId,
    elsewhere: yaar_store::WorkspaceId,
    first: String,
    second: String,
}

fn compared(scratch: &Scratch, kept: &Kept) -> Compared {
    let repo = scratch.repository("repo");
    // Several lines, so that git pairs the rename by what the file holds.
    let moved = "the file that is moved\nfrom one place\nto another\nand keeps its lines\n";
    write(&repo, "kept.txt", "kept\n");
    write(&repo, "removed.txt", "removed\n");
    write(&repo, "moved.txt", moved);
    commit_all(&repo, "files");
    let (workspace, worktree) = kept.open(&repo);
    let [a, b] = [1, 2].map(|n| {
        let record = SessionRecord {
            id: yaar_store::SessionId::new(format!("session-{n}")),
            workspace: workspace.clone(),
            agent: "claude".to_owned(),
            cli_session: format!("cli-{n}"),
            terminal: Some(format!("terminal-{n}")),
            number: n,
            running: true,
            ended: None,
            ended_by: None,
        };
        kept.store.put_session(&record).expect("keep the session");
        let review = kept.reviews.session_started(&record).expect("its review");
        ReviewId::new(review.as_str())
    });
    let first = kept.state(&workspace, &worktree);

    write(&repo, "kept.txt", "changed\n");
    write(&repo, "added-a.txt", "a\n");
    write(&repo, "added-b.txt", "b\n");
    write(&repo, "shared.txt", "shared\n");
    write(&repo, "nobody.txt", "nobody\n");
    std::fs::remove_file(repo.join("removed.txt")).expect("remove a file");
    std::fs::rename(repo.join("moved.txt"), repo.join("renamed.txt")).expect("move a file");
    let second = kept.state(&workspace, &worktree);
    // What the Keeper keeps at a turn's end: who is credited with each path. `nobody.txt`
    // is credited to no one, and so is not named.
    let by = |path: &str, sessions: &[&str]| yaar_review_engine::Credited {
        path: PathBuf::from(path),
        by: sessions
            .iter()
            .map(|session| (*session).to_owned())
            .collect(),
    };
    kept.reviews
        .keep_credits(
            &workspace,
            &second,
            &[
                by("added-a.txt", &["session-1"]),
                by("added-b.txt", &["session-2"]),
                by("kept.txt", &["session-1"]),
                by("removed.txt", &["session-2"]),
                by("renamed.txt", &["session-1"]),
                by("shared.txt", &["session-1", "session-2"]),
            ],
        )
        .expect("keep the credits");

    // The review of no agent of another workspace, which the first one's review is not of.
    let other = scratch.repository("other");
    let (elsewhere, _) = kept.open(&other);
    let review_elsewhere = kept
        .reviews
        .of_workspace(&elsewhere, None)
        .expect("its review")
        .reviews[0]
        .id()
        .clone();
    Compared {
        workspace,
        review_a: a,
        review_b: b,
        review_elsewhere: ReviewId::new(review_elsewhere.as_str()),
        elsewhere,
        first,
        second,
    }
}

fn compare_for_review(
    window: &mut Window,
    workspace: &yaar_store::WorkspaceId,
    review: &ReviewId,
    from: &str,
    to: &str,
) -> Result<Reply, Refusal> {
    window.ask(Request::CompareForReview {
        workspace: link_workspace(workspace),
        review: review.clone(),
        from: from.to_owned(),
        to: to.to_owned(),
    })
}

#[test]
fn a_review_lists_the_paths_of_its_session_and_those_no_agent_made_each_with_its_status_and_credits()
 {
    let scratch = Scratch::new();
    let kept = Kept::new(&scratch);
    let c = compared(&scratch, &kept);
    drop(kept);
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let (one, two) = ("session-1", "session-2");

    // The first session's review: its own paths and the one no agent made, a path both made
    // whole with both sessions, and nothing of the other session alone.
    assert_eq!(
        compare_for_review(&mut window, &c.workspace, &c.review_a, &c.first, &c.second),
        Ok(Reply::ComparedForReview(vec![
            credited("added-a.txt", FileStatus::Added, &[one]),
            credited("kept.txt", FileStatus::Modified, &[one]),
            credited("nobody.txt", FileStatus::Added, &[]),
            credited(
                "renamed.txt",
                FileStatus::Renamed {
                    from: PathBuf::from("moved.txt")
                },
                &[one]
            ),
            credited("shared.txt", FileStatus::Added, &[one, two]),
        ]))
    );
    assert_eq!(
        compare_for_review(&mut window, &c.workspace, &c.review_b, &c.first, &c.second),
        Ok(Reply::ComparedForReview(vec![
            credited("added-b.txt", FileStatus::Added, &[two]),
            credited("nobody.txt", FileStatus::Added, &[]),
            credited("removed.txt", FileStatus::Deleted, &[two]),
            credited("shared.txt", FileStatus::Added, &[one, two]),
        ]))
    );
    // The same version on both sides differs in nothing.
    assert_eq!(
        compare_for_review(&mut window, &c.workspace, &c.review_a, &c.second, &c.second),
        Ok(Reply::ComparedForReview(Vec::new()))
    );
    window.stop(keeper);
}

#[test]
fn a_comparison_for_a_review_is_refused_for_an_unknown_review_workspace_or_version_as_a_comparison_is()
 {
    let scratch = Scratch::new();
    let kept = Kept::new(&scratch);
    let c = compared(&scratch, &kept);
    drop(kept);
    let keeper = scratch.keeper();
    let mut window = scratch.window();

    let unknown = ReviewId::new("no-such-review");
    assert_eq!(
        compare_for_review(&mut window, &c.workspace, &unknown, &c.first, &c.second),
        Err(Refusal::UnknownReview { review: unknown })
    );
    // A review of another workspace names no change of this one.
    assert_eq!(
        compare_for_review(
            &mut window,
            &c.workspace,
            &c.review_elsewhere,
            &c.first,
            &c.second
        ),
        Err(Refusal::UnknownReview {
            review: c.review_elsewhere.clone()
        })
    );
    assert_eq!(
        compare_for_review(&mut window, &c.elsewhere, &c.review_a, &c.first, &c.second),
        Err(Refusal::UnknownReview {
            review: c.review_a.clone()
        })
    );
    let nobody = yaar_store::WorkspaceId::new("no-such-workspace");
    assert_eq!(
        compare_for_review(&mut window, &nobody, &c.review_a, &c.first, &c.second),
        Err(Refusal::UnknownWorkspace {
            workspace: link_workspace(&nobody)
        })
    );
    let missing = "0".repeat(40);
    for version in ["HEAD", "not-an-id", &missing] {
        let refused = Err(Refusal::UnknownRevision {
            revision: version.to_owned(),
        });
        assert_eq!(
            compare_for_review(&mut window, &c.workspace, &c.review_a, version, &c.second),
            refused,
            "{version} as the first"
        );
        assert_eq!(
            compare_for_review(&mut window, &c.workspace, &c.review_a, &c.first, version),
            refused,
            "{version} as the second"
        );
    }
    window.stop(keeper);
}

/// The reports of the Claude Code session `session` of the recording of two sessions in one
/// folder: its start, its prompt, a write of each of `paths` with its own tool, and the end
/// of its turn, as `writes_of` gives them, read as if the recording was made in `repo`.
fn claude_turn(session: usize, repo: &Path, paths: &[&str]) -> Vec<Vec<u8>> {
    let events = yaar_system_tests::recorded::hook_events("claude/two-sessions-files.jsonl");
    let mut sessions: Vec<&str> = Vec::new();
    for event in events.iter().filter(|event| event.event == "SessionStart") {
        if !sessions.contains(&event.session()) {
            sessions.push(event.session());
        }
    }
    let stop = events
        .iter()
        .find(|event| event.event == "Stop" && event.session() == sessions[session])
        .expect("the session's turn ends in the recording");
    let mut payloads = writes_of(session, repo, paths);
    payloads.push(stop.in_project(repo).input_bytes());
    payloads
}

/// The reports of a pi session, from the recording `pi/turn-tools.jsonl`: its start (line 1),
/// its message (53), a write with its own tool for each of `paths` (64, whose `input.path`
/// is the one field changed, since the recording wrote one file of its own), and the end of
/// its run (76), as if the recording was made in `repo`.
fn pi_turn(repo: &Path, paths: &[&str]) -> Vec<Vec<u8>> {
    let events = yaar_system_tests::recorded::pi_events("pi/turn-tools.jsonl");
    let of = |line: usize| events[line].in_project(repo);
    let mut payloads = vec![of(1).payload_bytes(), of(53).payload_bytes()];
    for path in paths {
        let mut write = of(64);
        assert_eq!(write.event, "tool_result");
        assert_eq!(write.payload["toolName"], "write");
        write.payload["input"]["path"] = (*path).into();
        payloads.push(write.payload_bytes());
    }
    payloads.push(of(76).payload_bytes());
    payloads
}

#[test]
fn a_claude_code_and_a_pi_session_in_one_worktree_are_each_shown_their_own_paths_those_no_agent_made_and_the_shared_one_with_both()
 {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let kept = Kept::new(&scratch);
    let (workspace, _) = kept.open(&repo);
    let from = git(&repo, &["rev-parse", "HEAD"]);
    drop(kept);
    for (name, text) in [
        ("claude-only.txt", "a\n"),
        ("pi-only.txt", "b\n"),
        ("both.txt", "both\n"),
        ("nobody.txt", "nobody\n"),
    ] {
        write(&repo, name, text);
    }
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    // Each session runs in a shell of the app of its own. The Windows that open them are
    // kept to the end: the output of a shell goes to the one that opened it.
    let size = TerminalSize {
        columns: 80,
        rows: 24,
    };
    let mut openers = Vec::new();
    let mut terminals: Vec<TerminalId> = Vec::new();
    for _ in 0..2 {
        let mut opener = scratch.window();
        let Ok(Reply::TerminalOpened { terminal }) = opener.ask(Request::OpenTerminal {
            workspace: link_workspace(&workspace),
            program: Program::Shell,
            size,
        }) else {
            panic!("no terminal");
        };
        terminals.push(terminal);
        openers.push(opener);
    }
    // Two processes that are alive for as long as the test runs: this one and its parent.
    // SAFETY: `getppid` takes no argument and cannot fail.
    let pids = [
        std::process::id(),
        unsafe { libc::getppid() }.unsigned_abs(),
    ];
    let reports = [
        (
            "claude",
            claude_turn(0, &repo, &["claude-only.txt", "both.txt"]),
        ),
        ("pi", pi_turn(&repo, &["pi-only.txt", "both.txt"])),
    ];
    // One connection for each report: a piece of a CLI leaves once it is answered, and
    // reads past the facts told before its answer.
    for (session, (agent, payloads)) in reports.into_iter().enumerate() {
        for payload in payloads {
            let answered = scratch.window().ask_past_facts(Request::Report {
                report: AgentReport {
                    agent: agent.to_owned(),
                    terminal: Some(terminals[session].clone()),
                    pid: pids[session],
                    payload,
                },
            });
            assert_eq!(answered, Ok(Reply::Done), "{agent}");
        }
    }
    // The barrier: both turns ended, each with a state and the credits kept with it.
    let ended = window.until_turns_ended(2);
    let newest = &ended[1].1;

    let Ok(Reply::Agents(agents)) = window.ask_past_facts(Request::ListAgents) else {
        panic!("no list of agents");
    };
    let sessions = &agents
        .iter()
        .find(|listed| listed.workspace == link_workspace(&workspace))
        .expect("the workspace has sessions")
        .sessions;
    let of = |agent: &str| {
        sessions
            .iter()
            .find(|session| session.agent == agent)
            .unwrap_or_else(|| panic!("no session of {agent}: {sessions:?}"))
    };
    let (claude, pi) = (of("claude"), of("pi"));
    let (review_claude, review_pi) = (
        claude
            .review
            .clone()
            .expect("Claude Code's session has a review"),
        pi.review.clone().expect("pi's session has a review"),
    );
    assert_ne!(review_claude, review_pi);
    let (c, p) = (claude.id.as_str(), pi.id.as_str());

    // From the commit the worktree started on, which the state store does not hold yet, to
    // the state the last turn ended on.
    let compare_review = |window: &mut Window, review: &ReviewId| {
        window.ask_past_facts(Request::CompareForReview {
            workspace: link_workspace(&workspace),
            review: review.clone(),
            from: from.clone(),
            to: newest.clone(),
        })
    };
    assert_eq!(
        compare_review(&mut window, &review_claude),
        Ok(Reply::ComparedForReview(vec![
            credited("both.txt", FileStatus::Added, &[c, p]),
            credited("claude-only.txt", FileStatus::Added, &[c]),
            credited("nobody.txt", FileStatus::Added, &[]),
        ]))
    );
    assert_eq!(
        compare_review(&mut window, &review_pi),
        Ok(Reply::ComparedForReview(vec![
            credited("both.txt", FileStatus::Added, &[c, p]),
            credited("nobody.txt", FileStatus::Added, &[]),
            credited("pi-only.txt", FileStatus::Added, &[p]),
        ]))
    );
    window.stop(keeper);
}

//! Component tests: the threads on the old side when the base of a workspace moves, as a
//! Window sees it through the Link, with a real repository, a real database and the Keeper
//! as the library on a thread of the test (`THREAD-09`, the part through the Keeper).
//!
//! The Keeper runs git with the environment of its process, so each test sets that
//! environment as its first statement, and these tests are run by `cargo nextest`, which
//! gives each test a process of its own.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_keeper::{Exit, Options, run};
use yaar_link::{
    Anchor, CommentId, DraftOn, Fact, Hello, Lines, Message, Refusal, Reply, Request, RequestId,
    ReviewId, Side, Thread, ThreadId, WorkspaceId, handshake, read_message, write_message,
};

/// How long a test waits for the Keeper before it fails: longer than the plan's ten
/// seconds, as every test file of this crate has it, since a Keeper starts git of its own
/// and a throttled machine runs it slowly.
const DEADLINE: Duration = Duration::from_secs(20);

/// A folder of the test's own with a short path, removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    /// Makes the folder and cleans the environment of this process for git: no
    /// configuration of the user or of the system, a fixed author and date, a home inside
    /// the folder. It has to be the first statement of a test.
    fn new() -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        // `/tmp` is a link on macOS, and the Keeper names a worktree by its resolved path.
        let root = Path::new("/tmp")
            .canonicalize()
            .expect("the temporary folder")
            .join(format!("yaar-keeper-base-moves-{}-{n}", std::process::id()));
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

    fn data(&self) -> PathBuf {
        self.0.join("data")
    }

    /// Starts the Keeper's library on a thread of this test, on the data folder of the
    /// test: a second call, after the first Keeper stopped, starts one on the same data.
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
                return Window {
                    stream,
                    next_id: 0,
                    facts: Vec::new(),
                    looked_at: 0,
                    replies: Vec::new(),
                };
            }
            assert!(
                Instant::now() < deadline,
                "no Keeper at {:?}",
                self.socket()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// A repository `repo` whose file `f` holds `line 1` to `line 6` on `main` (the commit
    /// `m1`), with the branches `mid` (`m2`, two lines put above line 3), `mid3` (`m3`, a
    /// line put at the top) and `feature` (`m3` and a second file), which is checked out.
    /// The base of its workspace is `main`, so its changes are counted from `m1`.
    fn history(&self) -> History {
        let repo = self.0.join("repo");
        std::fs::create_dir_all(&repo).expect("make the folder");
        git(&repo, &["init", "--quiet", "-b", "main"]);
        git(&repo, &["config", "user.name", "A Test"]);
        git(&repo, &["config", "user.email", "test@example.invalid"]);
        let mut lines: Vec<String> = (1..=6).map(|n| format!("line {n}")).collect();
        let commit = |lines: &[String], message: &str| {
            std::fs::write(
                repo.join("f"),
                lines.iter().map(|l| format!("{l}\n")).collect::<String>(),
            )
            .expect("write a file");
            git(&repo, &["add", "--all"]);
            git(&repo, &["commit", "--quiet", "-m", message]);
            git(&repo, &["rev-parse", "HEAD"])
        };
        let m1 = commit(&lines, "first");
        git(&repo, &["checkout", "--quiet", "-b", "mid"]);
        lines.splice(2..2, ["new 1".to_owned(), "new 2".to_owned()]);
        let m2 = commit(&lines, "two lines");
        git(&repo, &["checkout", "--quiet", "-b", "mid3"]);
        lines.insert(0, "top".to_owned());
        let m3 = commit(&lines, "a line at the top");
        git(&repo, &["checkout", "--quiet", "-b", "feature"]);
        std::fs::write(repo.join("g"), "g\n").expect("write a file");
        git(&repo, &["add", "--all"]);
        git(&repo, &["commit", "--quiet", "-m", "the feature"]);
        History { repo, m1, m2, m3 }
    }
}

/// The repository of [`Scratch::history`] and its commits.
struct History {
    repo: PathBuf,
    m1: String,
    m2: String,
    m3: String,
}

impl History {
    /// Moves `main` to `commit` while nobody has it checked out, as a pull does.
    fn move_main(&self, commit: &str) {
        git(&self.repo, &["branch", "--force", "main", commit]);
    }

    /// Where a reviewer writes a thread on line 3 of `f` on the old side of `commit`.
    fn on_line_3_of_the_old_side(&self, commit: &str) -> Anchor {
        Anchor {
            tree: Some(commit.to_owned()),
            path: PathBuf::from("f"),
            side: Side::Old,
            lines: Lines { first: 3, last: 3 },
            text: "line 3".to_owned(),
        }
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
        .expect("text")
        .trim_end()
        .to_owned()
}

/// One end of the Link, as a Window holds it. Everything it is sent is kept, so that a
/// test reads it in order, and what it has looked at is not looked at again.
struct Window {
    stream: UnixStream,
    next_id: u32,
    facts: Vec<Fact>,
    looked_at: usize,
    replies: Vec<(RequestId, Result<Reply, Refusal>, usize)>,
}

impl Window {
    fn send(&mut self, request: Request) -> RequestId {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        write_message(&mut self.stream, &Message::Request { id, request }).expect("send");
        id
    }

    /// Reads one frame and keeps it, a reply with the number of facts that came before it.
    /// Fails the test at the deadline, with the facts that came.
    fn read(&mut self, waiting_for: &str) {
        match read_message(&mut self.stream) {
            Ok(Message::Fact(fact)) => self.facts.push(fact),
            Ok(Message::Reply { to, reply }) => self.replies.push((to, reply, self.facts.len())),
            Ok(other) => panic!("waiting for {waiting_for}, a frame nobody sends: {other:?}"),
            Err(e) => panic!(
                "waiting for {waiting_for}: no frame from the Keeper ({e}); the facts so far: {:#?}",
                self.facts
            ),
        }
    }

    /// Sends a request and waits for its reply, which comes with how many facts this
    /// Window had been told when it came.
    fn ask_noting_facts(&mut self, request: Request) -> (Result<Reply, Refusal>, usize) {
        let id = self.send(request);
        loop {
            if let Some(at) = self.replies.iter().position(|(to, ..)| *to == id) {
                let (_, reply, facts) = self.replies.remove(at);
                return (reply, facts);
            }
            self.read("a reply");
        }
    }

    /// Sends a request and waits for its reply.
    fn ask(&mut self, request: Request) -> Result<Reply, Refusal> {
        self.ask_noting_facts(request).0
    }

    fn open(&mut self, folder: &Path) -> WorkspaceId {
        let opened = self.ask(Request::OpenFolder {
            path: folder.to_owned(),
        });
        let Ok(Reply::Opened { workspace, .. }) = opened else {
            panic!("not opened: {opened:?}");
        };
        workspace
    }

    /// How many `ReviewsChanged` this Window was told so far.
    fn reviews_changed(&self) -> usize {
        self.facts
            .iter()
            .filter(|fact| matches!(fact, Fact::ReviewsChanged { .. }))
            .count()
    }

    /// Waits for a `ReviewsChanged` at or after the fact numbered `from`: the facts told
    /// after the reply that came with `from` facts before it.
    fn wait_for_reviews_changed_from(&mut self, from: usize) {
        loop {
            if self.facts[from.min(self.facts.len())..]
                .iter()
                .any(|fact| matches!(fact, Fact::ReviewsChanged { .. }))
            {
                return;
            }
            self.read("a ReviewsChanged");
        }
    }

    /// What the changes of `workspace` are counted from.
    fn counted_from(&mut self, workspace: &WorkspaceId) -> String {
        match self.ask(Request::GetChanges {
            workspace: workspace.clone(),
        }) {
            Ok(Reply::Changes(changes)) => changes.counted_from.expect("a commit"),
            other => panic!("no changes: {other:?}"),
        }
    }

    /// The review of the workspace, of no agent, and its threads.
    fn threads(&mut self, workspace: &WorkspaceId) -> (ReviewId, Vec<Thread>) {
        match self.ask(Request::GetReviews {
            workspace: workspace.clone(),
        }) {
            Ok(Reply::Reviews(held)) => {
                let [review] = &held.reviews[..] else {
                    panic!("not one review: {:?}", held.reviews);
                };
                (review.id.clone(), review.threads.clone())
            }
            other => panic!("no reviews: {other:?}"),
        }
    }

    fn thread(&mut self, workspace: &WorkspaceId, thread: &ThreadId) -> Thread {
        self.threads(workspace)
            .1
            .into_iter()
            .find(|found| found.id == *thread)
            .expect("the thread is kept")
    }

    /// Saves the first comment of thread `n` on `anchor` and waits for the `ReviewsChanged`
    /// that follows the reply.
    fn start(&mut self, review: &ReviewId, n: u8, anchor: Anchor) {
        let (reply, facts) = self.ask_noting_facts(Request::SaveDraft {
            review: review.clone(),
            thread: thread_id(n),
            comment: CommentId::new(format!("{:02x}", n + 0x80).repeat(16)),
            on: DraftOn::NewThread { anchor },
            text: format!("comment {n}"),
        });
        assert!(matches!(reply, Ok(Reply::Done)), "{reply:?}");
        self.wait_for_reviews_changed_from(facts);
    }

    fn stop(mut self, keeper: JoinHandle<std::io::Result<Exit>>) {
        write_message(&mut self.stream, &Message::Shutdown).expect("ask for the stop");
        assert_eq!(keeper.join().expect("join").expect("run"), Exit::Shutdown);
    }
}

/// The id of thread `n`: 32 digits.
fn thread_id(n: u8) -> ThreadId {
    ThreadId::new(format!("{n:02x}").repeat(16))
}

/// Where the thread sits on the old side of `tree`, on `first` of `f`.
fn sits_on(thread: &Thread, tree: &str, first: u32) {
    let now = thread
        .now
        .as_ref()
        .unwrap_or_else(|| panic!("the thread has no place: {thread:?}"));
    assert_eq!(
        (now.tree.as_str(), now.side, now.lines, now.notice),
        (tree, Side::Old, Lines { first, last: first }, None),
        "{thread:?}"
    );
}

// THREAD-09
#[test]
fn a_base_set_to_a_branch_with_another_fork_point_moves_the_old_side_and_tells_after_the_reply() {
    let scratch = Scratch::new();
    let history = scratch.history();
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&history.repo);
    assert_eq!(window.counted_from(&workspace), history.m1);
    let (review, _) = window.threads(&workspace);
    window.start(&review, 1, history.on_line_3_of_the_old_side(&history.m1));
    let told = window.reviews_changed();

    let (reply, facts_before) = window.ask_noting_facts(Request::SetBase {
        workspace: workspace.clone(),
        base: "mid".to_owned(),
    });
    assert!(matches!(reply, Ok(Reply::Done)), "{reply:?}");
    window.wait_for_reviews_changed_from(facts_before);
    assert_eq!(window.reviews_changed(), told + 1);

    assert_eq!(window.counted_from(&workspace), history.m2);
    sits_on(&window.thread(&workspace, &thread_id(1)), &history.m2, 5);
    window.stop(keeper);
}

#[test]
fn a_base_that_moved_under_the_running_keeper_moves_the_threads_once_and_the_second_look_tells_nothing()
 {
    let scratch = Scratch::new();
    let history = scratch.history();
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&history.repo);
    assert_eq!(window.counted_from(&workspace), history.m1);
    let (review, _) = window.threads(&workspace);
    window.start(&review, 1, history.on_line_3_of_the_old_side(&history.m1));
    let told = window.reviews_changed();

    // A pull moves `main` to `m2`: the fork point of the workspace moves with it.
    history.move_main(&history.m2);
    assert_eq!(window.counted_from(&workspace), history.m2);
    assert_eq!(window.counted_from(&workspace), history.m2);
    // The barrier: a thread saved after the second answer tells its own `ReviewsChanged`
    // after that reply, so that nothing the second answer told can still be on its way.
    let new_side = Anchor {
        tree: None,
        path: PathBuf::from("g"),
        side: Side::New,
        lines: Lines { first: 1, last: 1 },
        text: "g".to_owned(),
    };
    window.start(&review, 2, new_side);

    // Told once for the first answer and once for the barrier.
    assert_eq!(window.reviews_changed(), told + 2);
    sits_on(&window.thread(&workspace, &thread_id(1)), &history.m2, 5);
    window.stop(keeper);
}

#[test]
fn a_base_that_moved_while_the_keeper_was_stopped_moves_the_threads_at_its_first_answer() {
    let scratch = Scratch::new();
    let history = scratch.history();
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&history.repo);
    assert_eq!(window.counted_from(&workspace), history.m1);
    let (review, _) = window.threads(&workspace);
    window.start(&review, 1, history.on_line_3_of_the_old_side(&history.m1));
    window.stop(keeper);

    history.move_main(&history.m2);
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let (reply, facts_before) = window.ask_noting_facts(Request::GetChanges {
        workspace: workspace.clone(),
    });
    let Ok(Reply::Changes(changes)) = reply else {
        panic!("no changes: {reply:?}");
    };
    assert_eq!(changes.counted_from.as_deref(), Some(history.m2.as_str()));
    window.wait_for_reviews_changed_from(facts_before);
    sits_on(&window.thread(&workspace, &thread_id(1)), &history.m2, 5);
    window.stop(keeper);
}

#[test]
fn a_base_set_after_a_silent_move_carries_the_threads_from_the_commit_last_seen() {
    let scratch = Scratch::new();
    let history = scratch.history();
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&history.repo);
    assert_eq!(window.counted_from(&workspace), history.m1);
    let (review, _) = window.threads(&workspace);
    window.start(&review, 1, history.on_line_3_of_the_old_side(&history.m1));

    // `main` moved and nobody asked for the changes since: the commit last seen is `m1`,
    // the one the base is on at the request is `m2`, and the base set is on `m3`.
    history.move_main(&history.m2);
    let (reply, facts_before) = window.ask_noting_facts(Request::SetBase {
        workspace: workspace.clone(),
        base: "mid3".to_owned(),
    });
    assert!(matches!(reply, Ok(Reply::Done)), "{reply:?}");
    window.wait_for_reviews_changed_from(facts_before);

    assert_eq!(window.counted_from(&workspace), history.m3);
    sits_on(&window.thread(&workspace, &thread_id(1)), &history.m3, 6);
    window.stop(keeper);
}

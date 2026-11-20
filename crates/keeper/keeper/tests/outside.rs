//! Component tests: a thread on a file outside the worktree, or one inside it that git
//! ignores, through the surface a Window uses, with a real repository the tests make and a
//! real database beneath. The thread is saved with no version and an absolute path, and the
//! Keeper gives it back with no version and the same path.
//!
//! The Keeper runs git as its user does, with the environment of its process. Each test
//! therefore cleans that environment as its first statement, and these tests are run by
//! `cargo nextest`, which gives each test a process of its own.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_keeper::{Exit, Options, run};
use yaar_link::{
    Anchor, CommentId, DraftOn, Fact, Hello, Lines, Message, PROTOCOL_VERSION, Refusal, Reply,
    Request, RequestId, ReviewId, Side, Thread, ThreadId, WorkspaceId, handshake, read_message,
    write_message,
};

/// What the one file of a new repository holds.
const FIVE_LINES: &str = "one\ntwo\nthree\nfour\nfive\n";

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
            .join(format!("yaar-keeper-outside-{}-{n}", std::process::id()));
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

    /// Starts the Keeper's library on a thread of this test.
    fn keeper(&self) -> JoinHandle<std::io::Result<Exit>> {
        let options = Options {
            socket: self.socket(),
            linger: Duration::from_secs(60),
            app_version: "test-keeper".to_owned(),
            data_dir: self.0.join("data"),
            companions: None,
            home: self.0.join("home"),
        };
        std::thread::spawn(move || run(options))
    }

    /// Attaches as a Window does, waiting for the Keeper to listen.
    fn window(&self) -> Window {
        let mine = Hello::for_this_process("test-window");
        assert_eq!(mine.protocol, PROTOCOL_VERSION);
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

    /// A repository in the folder `name` with one commit on `main`, and its file `a.txt`.
    fn repository(&self, name: &str) -> PathBuf {
        let folder = self.0.join(name);
        std::fs::create_dir_all(&folder).expect("make the folder");
        git(&folder, &["init", "--quiet", "-b", "main"]);
        std::fs::write(folder.join("a.txt"), FIVE_LINES).expect("write a file");
        std::fs::write(folder.join(".gitignore"), "ignored.log\n").expect("write the ignores");
        git(&folder, &["add", "--all"]);
        git(&folder, &["commit", "--quiet", "-m", "first"]);
        folder
    }

    /// A file in a folder of the scratch that no repository holds, five lines of `fn N`.
    fn library_file(&self) -> PathBuf {
        let folder = self.0.join("lib");
        std::fs::create_dir_all(&folder).expect("make the folder");
        let file = folder.join("x.rs");
        let lines: String = (1..=5).map(|n| format!("fn {n}\n")).collect();
        std::fs::write(&file, lines).expect("write the file");
        file
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn git(folder: &Path, args: &[&str]) {
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
}

/// One end of the Link, as a Window holds it.
struct Window {
    stream: UnixStream,
    next_id: u32,
}

impl Window {
    /// The next frame the Keeper sent, whatever it is. Fails the test at the deadline.
    fn next(&mut self) -> Message {
        read_message(&mut self.stream).expect("a frame from the Keeper")
    }

    /// Sends a request and expects its reply as the very next frame.
    fn ask(&mut self, request: Request) -> Result<Reply, Refusal> {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        write_message(&mut self.stream, &Message::Request { id, request }).expect("send");
        match self.next() {
            Message::Reply { to, reply } if to == id => reply,
            other => panic!("not the reply to {id:?}: {other:?}"),
        }
    }

    /// Expects a fact as the very next frame.
    fn fact(&mut self) -> Fact {
        match self.next() {
            Message::Fact(fact) => fact,
            other => panic!("not a fact: {other:?}"),
        }
    }

    /// Opens the folder, takes the list of workspaces that follows, and gives the
    /// workspace with the id of its review, of no agent.
    fn open(&mut self, folder: &Path) -> (WorkspaceId, ReviewId) {
        let Ok(Reply::Opened { workspace, .. }) = self.ask(Request::OpenFolder {
            path: folder.to_owned(),
        }) else {
            panic!("not opened");
        };
        assert!(matches!(self.fact(), Fact::Workspaces(_)));
        let review = self.threads(&workspace).0;
        (workspace, review)
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

    /// One thread, by its id.
    fn thread(&mut self, workspace: &WorkspaceId, thread: &ThreadId) -> Thread {
        self.threads(workspace)
            .1
            .into_iter()
            .find(|found| found.id == *thread)
            .expect("the thread is kept")
    }

    /// Saves the first comment of thread `n` on `lines` of `path`, as the Window sends it,
    /// and, when that is done, takes the fact that the reviews changed.
    fn start(&mut self, review: &ReviewId, n: u8, anchor: Anchor) -> Result<Reply, Refusal> {
        let (thread, comment) = ids(n);
        let saved = self.ask(Request::SaveDraft {
            review: review.clone(),
            thread,
            comment,
            on: DraftOn::NewThread { anchor },
            text: format!("comment {n}"),
        });
        if saved.is_ok() {
            assert!(matches!(self.fact(), Fact::ReviewsChanged { .. }));
        }
        saved
    }

    fn stop(mut self, keeper: JoinHandle<std::io::Result<Exit>>) {
        write_message(&mut self.stream, &Message::Shutdown).expect("ask for the stop");
        assert_eq!(keeper.join().expect("join").expect("run"), Exit::Shutdown);
    }
}

/// The ids of a thread and of its comment that the tests draw: 32 digits each, and no two
/// alike.
fn ids(n: u8) -> (ThreadId, CommentId) {
    (
        ThreadId::new(format!("{n:02x}").repeat(16)),
        CommentId::new(format!("{:02x}", n + 0x80).repeat(16)),
    )
}

/// Where a new thread sits when the Window saves it: line `line` of `path` on the new
/// side, with `code` as the text of the line.
fn anchor_on(tree: Option<&str>, path: &Path, line: u32, code: &str) -> Anchor {
    Anchor {
        tree: tree.map(str::to_owned),
        path: path.to_owned(),
        side: Side::New,
        lines: Lines {
            first: line,
            last: line,
        },
        text: code.to_owned(),
    }
}

#[test]
fn a_thread_on_a_file_outside_the_worktree_is_saved_with_no_version_and_given_back_with_none() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let library = scratch.library_file();
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let (workspace, review) = window.open(&repo);

    assert_eq!(
        window.start(&review, 1, anchor_on(None, &library, 3, "fn 3")),
        Ok(Reply::Done)
    );

    let (thread, _) = ids(1);
    let kept = window.thread(&workspace, &thread);
    assert_eq!(kept.anchor.tree, None);
    assert_eq!(kept.anchor.path, library);
    assert_eq!(kept.anchor.side, Side::New);
    assert_eq!(kept.anchor.lines, Lines { first: 3, last: 3 });
    assert_eq!(kept.anchor.text, "fn 3");
    assert_eq!(kept.comments[0].text, "comment 1");
    assert_eq!(kept.now, None);
    window.stop(keeper);
}

#[test]
fn a_thread_on_a_file_of_the_worktree_still_records_a_state_and_the_outside_thread_is_not_carried_to_it()
 {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let library = scratch.library_file();
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let (workspace, review) = window.open(&repo);
    assert_eq!(
        window.start(&review, 1, anchor_on(None, &library, 3, "fn 3")),
        Ok(Reply::Done)
    );

    assert_eq!(
        window.start(&review, 2, anchor_on(None, Path::new("a.txt"), 3, "three")),
        Ok(Reply::Done)
    );

    let (inside, _) = ids(2);
    let recorded = window
        .thread(&workspace, &inside)
        .anchor
        .tree
        .expect("a thread of the worktree sits on a state");
    assert_eq!(recorded.len(), 40, "{recorded}");
    let (outside, _) = ids(1);
    let outside = window.thread(&workspace, &outside);
    assert_eq!(outside.anchor.tree, None);
    assert_eq!(outside.now, None, "no place, no notice");
    window.stop(keeper);
}

#[test]
fn a_thread_outside_the_worktree_is_kept_when_the_worktree_is_gone() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let library = scratch.library_file();
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let (workspace, review) = window.open(&repo);
    std::fs::remove_dir_all(&repo).expect("remove the worktree");

    assert_eq!(
        window.start(&review, 1, anchor_on(None, &library, 2, "fn 2")),
        Ok(Reply::Done)
    );
    // Lines of the files on disk need the worktree, as in slice 3.
    let refused = window.start(&review, 2, anchor_on(None, Path::new("a.txt"), 1, "one"));
    assert!(
        matches!(&refused, Err(Refusal::Failed { reason }) if reason.contains("worktree is gone")),
        "{refused:?}"
    );

    let (thread, _) = ids(1);
    assert_eq!(window.thread(&workspace, &thread).anchor.path, library);
    window.stop(keeper);
}

#[test]
fn a_path_that_is_absolute_and_inside_the_worktree_is_given_back_as_it_was_saved() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let ignored = repo.join("ignored.log");
    std::fs::write(&ignored, "first\nsecond\n").expect("write an ignored file");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let (workspace, review) = window.open(&repo);

    assert_eq!(
        window.start(&review, 1, anchor_on(None, &ignored, 2, "second")),
        Ok(Reply::Done)
    );

    let (thread, _) = ids(1);
    let kept = window.thread(&workspace, &thread);
    assert_eq!(kept.anchor.tree, None);
    assert_eq!(kept.anchor.path, ignored, "not made relative");
    window.stop(keeper);
}

#[test]
fn a_version_named_with_an_absolute_path_is_not_read_and_the_thread_comes_back_with_none() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let library = scratch.library_file();
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let (workspace, review) = window.open(&repo);
    // A commit the Keeper never held: keeping it would fail.
    let unheld = "1".repeat(40);

    assert_eq!(
        window.start(&review, 1, anchor_on(Some(&unheld), &library, 1, "fn 1")),
        Ok(Reply::Done)
    );

    let (thread, _) = ids(1);
    assert_eq!(window.thread(&workspace, &thread).anchor.tree, None);
    window.stop(keeper);
}

#[test]
fn the_review_copied_as_text_names_the_absolute_path_and_the_text() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let library = scratch.library_file();
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let (_, review) = window.open(&repo);
    assert_eq!(
        window.start(&review, 1, anchor_on(None, &library, 4, "fn 4")),
        Ok(Reply::Done)
    );

    let Ok(Reply::Copied { text, round }) = window.ask(Request::CopyReview {
        review: review.clone(),
    }) else {
        panic!("not copied");
    };

    assert_eq!(round, 1);
    assert!(
        text.contains(&format!("{}, line 4 on the new side", library.display())),
        "{text}"
    );
    assert!(text.contains("fn 4"), "{text}");
    assert!(text.contains("comment 1"), "{text}");
    window.stop(keeper);
}

#[test]
fn a_thread_outside_the_worktree_is_still_there_with_no_version_after_the_keeper_restarts() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let library = scratch.library_file();
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let (workspace, review) = window.open(&repo);
    assert_eq!(
        window.start(&review, 1, anchor_on(None, &library, 5, "fn 5")),
        Ok(Reply::Done)
    );
    window.stop(keeper);

    let keeper = scratch.keeper();
    let mut again = scratch.window();
    let (thread, _) = ids(1);
    let kept = again.thread(&workspace, &thread);
    assert_eq!(kept.anchor.tree, None);
    assert_eq!(kept.anchor.path, library);
    assert_eq!(kept.anchor.text, "fn 5");
    again.stop(keeper);
}

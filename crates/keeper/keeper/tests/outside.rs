//! Each test cleans the process environment first, so each needs a process of its own (nextest).

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

const FIVE_LINES: &str = "one\ntwo\nthree\nfour\nfive\n";

const DEADLINE: Duration = Duration::from_secs(20);

struct Scratch(PathBuf);

impl Scratch {
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
        // SAFETY: the first statement of the test; nextest runs each test in a process of its own,
        // so no other thread exists.
        unsafe {
            for (name, value) in settings {
                std::env::set_var(name, value);
            }
            std::env::set_var("HOME", root.join("home"));
            // Where the Keeper's copies of git's file lists go.
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

    fn keeper(&self) -> JoinHandle<std::io::Result<Exit>> {
        let options = Options {
            socket: self.socket(),
            linger: Duration::from_secs(60),
            app_version: "test-keeper".to_owned(),
            data_dir: self.0.join("data"),
            companions: None,
            home: self.0.join("home"),
            login_shell: None,
        };
        std::thread::spawn(move || run(options))
    }

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
    git_prints(folder, args);
}

fn git_prints(folder: &Path, args: &[&str]) -> String {
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

struct Window {
    stream: UnixStream,
    next_id: u32,
}

impl Window {
    fn next(&mut self) -> Message {
        read_message(&mut self.stream).expect("a frame from the Keeper")
    }

    fn ask(&mut self, request: Request) -> Result<Reply, Refusal> {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        write_message(&mut self.stream, &Message::Request { id, request }).expect("send");
        match self.next() {
            Message::Reply { to, reply } if to == id => reply,
            other => panic!("not the reply to {id:?}: {other:?}"),
        }
    }

    fn fact(&mut self) -> Fact {
        match self.next() {
            Message::Fact(fact) => fact,
            other => panic!("not a fact: {other:?}"),
        }
    }

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

    /// Saves the first comment of thread `n` and, when that succeeds, takes the fact that the
    /// reviews changed.
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

fn ids(n: u8) -> (ThreadId, CommentId) {
    (
        ThreadId::new(format!("{n:02x}").repeat(16)),
        CommentId::new(format!("{:02x}", n + 0x80).repeat(16)),
    )
}

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
fn a_thread_on_a_file_outside_the_worktree_saved_on_the_old_side_is_given_back_on_the_new() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let library = scratch.library_file();
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let (workspace, review) = window.open(&repo);
    let on_old_side = Anchor {
        side: Side::Old,
        ..anchor_on(None, &library, 3, "fn 3")
    };

    assert_eq!(window.start(&review, 1, on_old_side), Ok(Reply::Done));

    let (thread, _) = ids(1);
    let kept = window.thread(&workspace, &thread);
    assert_eq!(kept.anchor.side, Side::New, "a file outside has one side");
    assert_eq!(kept.anchor.path, library);
    window.stop(keeper);
}

#[test]
fn a_thread_on_a_file_of_the_worktree_still_records_a_state_and_is_carried_while_the_outside_thread_has_no_place()
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
    assert_ne!(
        recorded,
        git_prints(&repo, &["rev-parse", "HEAD"]),
        "a state of the files on disk, not the commit"
    );

    // The inside thread moving shows the carrying ran; only the engine's own tests show the outside
    // one is never a candidate.
    std::fs::write(repo.join("a.txt"), format!("zero\n{FIVE_LINES}")).expect("change the file");
    assert_eq!(
        window.start(&review, 3, anchor_on(None, Path::new("a.txt"), 1, "zero")),
        Ok(Reply::Done)
    );
    let (third, _) = ids(3);
    let second = window
        .thread(&workspace, &third)
        .anchor
        .tree
        .expect("the third thread sits on the second state");
    assert_ne!(second, recorded);
    let moved = window
        .thread(&workspace, &inside)
        .now
        .expect("the thread of the worktree was carried");
    assert_eq!(moved.tree, second);
    assert_eq!(moved.lines, Lines { first: 4, last: 4 });

    let (outside, _) = ids(1);
    let outside = window.thread(&workspace, &outside);
    assert_eq!(outside.anchor.tree, None);
    assert_eq!(outside.anchor.lines, Lines { first: 3, last: 3 });
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
fn a_thread_outside_the_worktree_is_saved_though_the_worktree_cannot_be_looked_up() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let library = scratch.library_file();
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let (workspace, review) = window.open(&repo);
    // The id file becomes a folder: the worktree lookup fails with an error of its own, and the
    // worktree is not gone.
    let id_file = repo.join(".git").join(yaar_workspaces::ID_FILE);
    let id = std::fs::read(&id_file).expect("read the id");
    std::fs::remove_file(&id_file).expect("remove the id file");
    std::fs::create_dir(&id_file).expect("make a folder of it");

    assert_eq!(
        window.start(&review, 1, anchor_on(None, &library, 2, "fn 2")),
        Ok(Reply::Done),
        "no lookup is made for a file outside the worktree"
    );
    // Control: a thread of the files on disk looks the worktree up and is refused with that error.
    let refused = window.start(&review, 2, anchor_on(None, Path::new("a.txt"), 1, "one"));
    assert!(
        matches!(&refused, Err(Refusal::Failed { reason }) if !reason.contains("worktree is gone")),
        "{refused:?}"
    );

    std::fs::remove_dir(&id_file).expect("remove the folder");
    std::fs::write(&id_file, id).expect("write the id back");
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

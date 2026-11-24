//! Each test sets the process environment first, so each needs a process of its own (nextest); the
//! restart test runs its first Keeper in a process of its own as well.

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_keeper::{Exit, Options, run};
use yaar_link::{
    Anchor, CommentId, DraftOn, Fact, Hello, Lines, Message, Refusal, Reply, Request, RequestId,
    ReviewId, Side, Thread, ThreadId, WorkspaceId, handshake, read_message, write_message,
};

const DEADLINE: Duration = Duration::from_secs(20);

const APART: &str = "YAAR_BASE_MOVES_KEEPER_IN";

const APART_TEST: &str = "the_keeper_of_a_process_of_its_own_serves_the_folder_its_variable_names_and_does_nothing_without_it";

struct Scratch(PathBuf);

impl Scratch {
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
        // SAFETY: the first statement of the test; nextest runs each test in a process of its own,
        // so no other thread exists.
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

    fn options(root: &Path) -> Options {
        Options {
            socket: root.join("k.sock"),
            linger: Duration::from_secs(60),
            app_version: "test-keeper".to_owned(),
            data_dir: root.join("data"),
            companions: None,
            home: root.join("home"),
            login_shell: None,
        }
    }

    fn keeper(&self) -> JoinHandle<std::io::Result<Exit>> {
        let options = Scratch::options(&self.0);
        std::thread::spawn(move || run(options))
    }

    fn keeper_apart(&self) -> Apart {
        let mut child = Command::new(std::env::current_exe().expect("the test program's path"))
            .args([APART_TEST, "--exact", "--nocapture", "--test-threads=1"])
            .env(APART, &self.0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .expect("start the Keeper's process");
        let stdout = child.stdout.take().expect("piped output");
        let (sender, lines) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Apart {
            child,
            lines,
            reader: Some(reader),
        }
    }

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

    /// `f` holds `line 1` to `line 6` on `main` (`m1`); branches `mid` (`m2`, two lines above line
    /// 3), `mid3` (`m3`, a line at the top) and `feature` (`m3` and a second file, checked out).
    /// The base is `main`.
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

struct Apart {
    child: Child,
    lines: Receiver<String>,
    reader: Option<JoinHandle<()>>,
}

impl Apart {
    fn ended(mut self) {
        loop {
            match self.lines.recv_timeout(DEADLINE) {
                Ok(line) if line == "stopped" => break,
                Ok(_) => {}
                Err(why) => panic!("the Keeper's process never reported `stopped`: {why}"),
            }
        }
        let status = self.child.wait().expect("wait for the Keeper's process");
        assert!(status.success(), "the Keeper's process ended with {status}");
    }
}

impl Drop for Apart {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[test]
fn the_keeper_of_a_process_of_its_own_serves_the_folder_its_variable_names_and_does_nothing_without_it()
 {
    let Some(root) = std::env::var_os(APART) else {
        return;
    };
    // The test program writes `test <name> ... ` without ending the line.
    println!();
    let exit = run(Scratch::options(Path::new(&root))).expect("run");
    assert_eq!(exit, Exit::Shutdown);
    println!("stopped");
}

struct History {
    repo: PathBuf,
    m1: String,
    m2: String,
    m3: String,
}

impl History {
    fn move_main(&self, commit: &str) {
        self.move_branch("main", commit);
    }

    fn move_branch(&self, name: &str, commit: &str) {
        git(&self.repo, &["branch", "--force", name, commit]);
    }

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

struct Window {
    stream: UnixStream,
    next_id: u32,
    facts: Vec<Fact>,
    replies: Vec<(RequestId, Result<Reply, Refusal>, usize)>,
}

impl Window {
    fn send(&mut self, request: Request) -> RequestId {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        write_message(&mut self.stream, &Message::Request { id, request }).expect("send");
        id
    }

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

    fn reviews_changed(&self) -> usize {
        self.facts
            .iter()
            .filter(|fact| matches!(fact, Fact::ReviewsChanged { .. }))
            .count()
    }

    fn wait_for_reviews_changed_from(&mut self, from: usize) {
        let deadline = Instant::now() + DEADLINE;
        loop {
            if self.facts[from.min(self.facts.len())..]
                .iter()
                .any(|fact| matches!(fact, Fact::ReviewsChanged { .. }))
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "no ReviewsChanged after the fact numbered {from}: {:#?}",
                self.facts
            );
            self.read("a ReviewsChanged");
        }
    }

    fn reviews_changed_between(&self, from: usize, to: usize) -> usize {
        self.facts[from..to]
            .iter()
            .filter(|fact| matches!(fact, Fact::ReviewsChanged { .. }))
            .count()
    }

    fn counted_from_noting_facts(&mut self, workspace: &WorkspaceId) -> (String, usize) {
        match self.ask_noting_facts(Request::GetChanges {
            workspace: workspace.clone(),
        }) {
            (Ok(Reply::Changes(changes)), facts) => {
                (changes.counted_from.expect("a commit"), facts)
            }
            (other, _) => panic!("no changes: {other:?}"),
        }
    }

    fn counted_from(&mut self, workspace: &WorkspaceId) -> String {
        self.counted_from_noting_facts(workspace).0
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

    fn start(&mut self, review: &ReviewId, n: u8, anchor: Anchor) -> usize {
        let (reply, facts) = self.ask_noting_facts(Request::SaveDraft {
            review: review.clone(),
            thread: thread_id(n),
            comment: CommentId::new(format!("{:02x}", n + 0x80).repeat(16)),
            on: DraftOn::NewThread { anchor },
            text: format!("comment {n}"),
        });
        assert!(matches!(reply, Ok(Reply::Done)), "{reply:?}");
        self.wait_for_reviews_changed_from(facts);
        facts
    }

    fn stop(mut self, keeper: JoinHandle<std::io::Result<Exit>>) {
        write_message(&mut self.stream, &Message::Shutdown).expect("ask for the stop");
        assert_eq!(keeper.join().expect("join").expect("run"), Exit::Shutdown);
    }

    fn stop_apart(mut self, keeper: Apart) {
        write_message(&mut self.stream, &Message::Shutdown).expect("ask for the stop");
        keeper.ended();
    }
}

fn thread_id(n: u8) -> ThreadId {
    ThreadId::new(format!("{n:02x}").repeat(16))
}

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

    history.move_main(&history.m2);
    let (counted, first) = window.counted_from_noting_facts(&workspace);
    assert_eq!(counted, history.m2);
    let (counted, second) = window.counted_from_noting_facts(&workspace);
    assert_eq!(counted, history.m2);
    // The barrier: a thread saved after the second answer tells its own `ReviewsChanged` after that
    // reply, so nothing the second answer told is still on its way.
    let new_side = Anchor {
        tree: None,
        path: PathBuf::from("g"),
        side: Side::New,
        lines: Lines { first: 1, last: 1 },
        text: "g".to_owned(),
    };
    let barrier = window.start(&review, 2, new_side);

    assert_eq!(window.reviews_changed_between(first, second), 1);
    assert_eq!(window.reviews_changed_between(second, barrier), 0);
    assert_eq!(window.reviews_changed(), told + 2);
    sits_on(&window.thread(&workspace, &thread_id(1)), &history.m2, 5);

    history.move_main(&history.m3);
    let (counted, moved) = window.counted_from_noting_facts(&workspace);
    assert_eq!(counted, history.m3);
    window.wait_for_reviews_changed_from(moved);
    sits_on(&window.thread(&workspace, &thread_id(1)), &history.m3, 6);
    window.stop(keeper);
}

#[test]
fn a_base_that_moved_while_the_keeper_was_stopped_moves_the_threads_at_its_first_answer() {
    let scratch = Scratch::new();
    let history = scratch.history();
    // The first Keeper is a process of its own: the note of the commit the changes are counted from
    // has to be in the data folder for the second to find it.
    let keeper = scratch.keeper_apart();
    let mut window = scratch.window();
    let workspace = window.open(&history.repo);
    assert_eq!(window.counted_from(&workspace), history.m1);
    let (review, _) = window.threads(&workspace);
    window.start(&review, 1, history.on_line_3_of_the_old_side(&history.m1));
    window.stop_apart(keeper);

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
fn a_base_set_before_any_look_at_the_changes_carries_the_threads_from_the_commit_before_it() {
    let scratch = Scratch::new();
    let history = scratch.history();
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&history.repo);
    let (review, _) = window.threads(&workspace);
    window.start(&review, 1, history.on_line_3_of_the_old_side(&history.m1));

    let (reply, facts_before) = window.ask_noting_facts(Request::SetBase {
        workspace: workspace.clone(),
        base: "mid".to_owned(),
    });
    assert!(matches!(reply, Ok(Reply::Done)), "{reply:?}");
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

    // `main` moved and nobody asked for the changes since: the commit last seen is `m1`, the base
    // at the request `m2`, and the base set is on `m3`.
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

#[test]
fn a_base_set_and_then_moved_by_a_pull_carries_the_threads_again_from_where_the_set_put_them() {
    let scratch = Scratch::new();
    let history = scratch.history();
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&history.repo);
    assert_eq!(window.counted_from(&workspace), history.m1);
    let (review, _) = window.threads(&workspace);
    window.start(&review, 1, history.on_line_3_of_the_old_side(&history.m1));

    let (reply, facts_before) = window.ask_noting_facts(Request::SetBase {
        workspace: workspace.clone(),
        base: "mid".to_owned(),
    });
    assert!(matches!(reply, Ok(Reply::Done)), "{reply:?}");
    window.wait_for_reviews_changed_from(facts_before);
    sits_on(&window.thread(&workspace, &thread_id(1)), &history.m2, 5);

    history.move_branch("mid", &history.m3);
    let (counted, moved) = window.counted_from_noting_facts(&workspace);
    assert_eq!(counted, history.m3);
    window.wait_for_reviews_changed_from(moved);
    sits_on(&window.thread(&workspace, &thread_id(1)), &history.m3, 6);
    window.stop(keeper);
}

#[test]
fn a_base_set_that_carried_nothing_is_noted_so_that_the_next_pull_carries_from_it() {
    let scratch = Scratch::new();
    let history = scratch.history();
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&history.repo);
    assert_eq!(window.counted_from(&workspace), history.m1);

    let (reply, _) = window.ask_noting_facts(Request::SetBase {
        workspace: workspace.clone(),
        base: "mid".to_owned(),
    });
    assert!(matches!(reply, Ok(Reply::Done)), "{reply:?}");

    // With `m1` still noted the answer would carry from `m1` and leave this thread behind.
    let (review, _) = window.threads(&workspace);
    let anchor = Anchor {
        text: "new 1".to_owned(),
        ..history.on_line_3_of_the_old_side(&history.m2)
    };
    window.start(&review, 1, anchor);
    history.move_branch("mid", &history.m3);
    let (counted, moved) = window.counted_from_noting_facts(&workspace);
    assert_eq!(counted, history.m3);
    sits_on(&window.thread(&workspace, &thread_id(1)), &history.m3, 4);
    window.wait_for_reviews_changed_from(moved);
    window.stop(keeper);
}

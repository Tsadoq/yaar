//! Component tests of the watched worktrees, with real repositories and the system's file notices.
//! Each test sets the process environment first, so nextest runs it in a process of its own.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_keeper::{Exit, Options, run};
use yaar_link::{
    Anchor, CommentId, DraftOn, Fact, Hello, Lines, Message, Refusal, Reply, Request, RequestId,
    ReviewId, Side, ThreadId, WorkspaceId, handshake, read_message, write_message,
};

mod support;

use support::{Environment, Scratch, git};

const DEADLINE: Duration = Duration::from_secs(20);

/// How long without a fact means none is coming: far more than the system takes to tell
/// of a file change (about 12 ms on macOS) and than the Keeper waits between two facts
/// of one workspace (250 ms).
const QUIET: Duration = Duration::from_millis(1200);

impl Scratch {
    fn new() -> Scratch {
        Scratch::in_tmp("watching", Environment::Git)
    }

    fn socket(&self) -> PathBuf {
        self.0.join("k.sock")
    }

    fn data_dir(&self) -> PathBuf {
        self.0.join("data")
    }

    fn keeper(&self) -> JoinHandle<std::io::Result<Exit>> {
        let options = Options {
            socket: self.socket(),
            linger: Duration::from_secs(60),
            app_version: "test-keeper".to_owned(),
            data_dir: self.data_dir(),
            companions: None,
            home: self.0.join("home"),
            login_shell: None,
        };
        std::thread::spawn(move || run(options))
    }

    /// Waits until the Keeper serves this Window, which the first reply proves: a fact told
    /// before then never reaches this Window.
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
                let mut window = Window { stream, next_id: 0 };
                window
                    .ask(Request::ListClis)
                    .expect("the Keeper serves the Window");
                return window;
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
        std::fs::write(folder.join("a.txt"), "one\n").expect("write a file");
        git(&folder, &["add", "--all"]);
        git(&folder, &["commit", "--quiet", "-m", "first"]);
        folder
    }

    fn states_of(&self, workspace: &WorkspaceId) -> usize {
        let store = self
            .data_dir()
            .join("states")
            .join(format!("{workspace}.git"));
        if !store.exists() {
            return 0;
        }
        let output = Command::new("git")
            .arg("--git-dir")
            .arg(&store)
            .args(["for-each-ref", "refs/yaar/states"])
            .stdin(Stdio::null())
            .output()
            .expect("start git");
        String::from_utf8_lossy(&output.stdout).lines().count()
    }

    fn file_in_state(&self, workspace: &WorkspaceId, state: &str, path: &str) -> String {
        let store = self
            .data_dir()
            .join("states")
            .join(format!("{workspace}.git"));
        let output = Command::new("git")
            .arg("--git-dir")
            .arg(&store)
            .args(["show", &format!("{state}:{path}")])
            .stdin(Stdio::null())
            .output()
            .expect("start git");
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// The Keeper records a state after its watcher runs, so no change made after this is missed.
    fn wait_until_watched(&self, workspace: &WorkspaceId) {
        let deadline = Instant::now() + DEADLINE;
        while self.states_of(workspace) == 0 {
            assert!(
                Instant::now() < deadline,
                "no state of {workspace} was recorded"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

struct Window {
    stream: UnixStream,
    next_id: u32,
}

impl Window {
    fn ask(&mut self, request: Request) -> Result<Reply, Refusal> {
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

    fn open(&mut self, folder: &Path) -> WorkspaceId {
        let opened = self.ask(Request::OpenFolder {
            path: folder.to_owned(),
        });
        let Ok(Reply::Opened { workspace, .. }) = opened else {
            panic!("not opened: {opened:?}");
        };
        workspace
    }

    fn show(&mut self, workspace: Option<&WorkspaceId>) -> Result<Reply, Refusal> {
        self.ask(Request::ShowWorkspace {
            workspace: workspace.cloned(),
        })
    }

    fn fact_within(&mut self, limit: Duration) -> Option<Fact> {
        self.stream
            .set_read_timeout(Some(limit))
            .expect("set the deadline of a read");
        let read = read_message(&mut self.stream);
        self.stream
            .set_read_timeout(Some(DEADLINE))
            .expect("set the deadline of a read");
        match read {
            Ok(Message::Fact(fact)) => Some(fact),
            Ok(other) => panic!("waiting for a fact, got {other:?}"),
            Err(yaar_link::LinkError::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                None
            }
            Err(e) => panic!("no fact: {e}"),
        }
    }

    fn wait_for(&mut self, wanted: impl Fn(&Fact) -> bool) -> Fact {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.fact_within(left.max(Duration::from_millis(1))) {
                Some(fact) if wanted(&fact) => return fact,
                Some(_) => {}
                None => panic!("the fact did not come"),
            }
        }
    }

    fn facts_until_quiet(&mut self) -> Vec<(Instant, Fact)> {
        let mut facts = Vec::new();
        while let Some(fact) = self.fact_within(QUIET) {
            facts.push((Instant::now(), fact));
        }
        facts
    }

    fn thread_states(&mut self, workspace: &WorkspaceId) -> (ReviewId, Vec<String>) {
        let Ok(Reply::Reviews(held)) = self.ask(Request::GetReviews {
            workspace: workspace.clone(),
        }) else {
            panic!("no reviews");
        };
        let review = held.reviews[0].clone();
        let states = review
            .threads
            .iter()
            .map(|thread| {
                thread
                    .anchor
                    .tree
                    .clone()
                    .expect("a thread sits on a version")
            })
            .collect();
        (review.id, states)
    }

    fn new_thread(&mut self, review: &ReviewId, n: u8) {
        self.new_thread_on(review, n, 1, "one");
    }

    fn new_thread_on(&mut self, review: &ReviewId, n: u8, line: u32, text: &str) {
        let anchor = Anchor {
            tree: None,
            path: PathBuf::from("a.txt"),
            side: Side::New,
            lines: Lines {
                first: line,
                last: line,
            },
            text: text.to_owned(),
        };
        let saved = self.ask(Request::SaveDraft {
            review: review.clone(),
            thread: ThreadId::new(format!("{n:02x}").repeat(16)),
            comment: CommentId::new(format!("{:02x}", n + 0x80).repeat(16)),
            on: DraftOn::NewThread { anchor },
            text: "a comment".to_owned(),
        });
        assert_eq!(saved, Ok(Reply::Done));
    }

    fn stop(mut self, keeper: JoinHandle<std::io::Result<Exit>>) {
        write_message(&mut self.stream, &Message::Shutdown).expect("ask for the stop");
        assert_eq!(keeper.join().expect("join").expect("run"), Exit::Shutdown);
    }
}

fn changes_of(workspace: &WorkspaceId) -> impl Fn(&Fact) -> bool + '_ {
    move |fact| matches!(fact, Fact::ChangesChanged { workspace: told } if told == workspace)
}

#[test]
fn a_workspace_the_keeper_does_not_hold_cannot_be_shown_but_none_can() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo);

    let unknown = WorkspaceId::new("f".repeat(32));
    assert_eq!(
        window.show(Some(&unknown)),
        Err(Refusal::UnknownWorkspace { workspace: unknown })
    );
    assert_eq!(window.show(Some(&workspace)), Ok(Reply::Done));
    assert_eq!(window.show(None), Ok(Reply::Done));
    window.stop(keeper);
}

#[test]
fn a_change_of_a_file_of_a_shown_worktree_is_told_to_every_window_and_the_first_watching_takes_a_state()
 {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut shows = scratch.window();
    let workspace = shows.open(&repo);
    assert_eq!(scratch.states_of(&workspace), 0, "nothing was watched yet");

    assert_eq!(shows.show(Some(&workspace)), Ok(Reply::Done));
    scratch.wait_until_watched(&workspace);
    let mut other = scratch.window();
    std::fs::write(repo.join("a.txt"), "one\ntwo\n").expect("change a file");
    other.wait_for(changes_of(&workspace));
    shows.wait_for(changes_of(&workspace));

    assert_eq!(other.show(Some(&workspace)), Ok(Reply::Done));
    assert_eq!(scratch.states_of(&workspace), 1);
    shows.stop(keeper);
}

#[test]
fn a_new_thread_on_the_files_on_disk_records_a_state_only_when_a_file_changed_since_the_newest() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    // A state is recorded through the repository's own filters, as `git add` does: the
    // untracked file runs the filter on every recording, so the lines it leaves count them.
    let count = scratch.0.join("recordings");
    git(
        &repo,
        &[
            "config",
            "filter.count.clean",
            &format!("echo x >> '{}'; cat", count.display()),
        ],
    );
    std::fs::write(repo.join(".gitattributes"), "counted.txt filter=count\n").expect("write it");
    git(&repo, &["add", ".gitattributes"]);
    git(&repo, &["commit", "--quiet", "-m", "attributes"]);
    std::fs::write(repo.join("counted.txt"), "counted\n").expect("write a file");
    let recordings = || {
        std::fs::read_to_string(&count)
            .map(|lines| lines.lines().count())
            .unwrap_or(0)
    };

    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    assert_eq!(window.show(Some(&workspace)), Ok(Reply::Done));
    scratch.wait_until_watched(&workspace);
    let first = recordings();
    assert!(first > 0, "the first watching recorded a state");

    let (review, _) = window.thread_states(&workspace);
    window.new_thread(&review, 1);
    window.new_thread(&review, 2);
    let (_, sitting) = window.thread_states(&workspace);
    assert_eq!(sitting.len(), 2);
    assert_eq!(sitting[0], sitting[1], "one state for both threads");
    assert_eq!(scratch.states_of(&workspace), 1);
    assert_eq!(recordings(), first, "no state was recorded for them");

    std::fs::write(repo.join("a.txt"), "one\ntwo\n").expect("change a file");
    window.wait_for(changes_of(&workspace));
    window.new_thread(&review, 3);
    let (_, sitting) = window.thread_states(&workspace);
    assert_eq!(sitting.len(), 3);
    assert_ne!(sitting[2], sitting[0], "a new state for the changed files");
    assert_eq!(scratch.states_of(&workspace), 2);
    assert!(recordings() > first, "a state was recorded for it");
    window.stop(keeper);
}

#[test]
fn a_new_thread_on_lines_a_write_just_made_sits_on_a_state_that_holds_them() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    assert_eq!(window.show(Some(&workspace)), Ok(Reply::Done));
    scratch.wait_until_watched(&workspace);
    let (review, _) = window.thread_states(&workspace);

    // A Window reads the write from disk before the watcher tells of it, so the thread must not
    // wait for that fact: the newest state lacks the line it comments on.
    std::fs::write(repo.join("a.txt"), "one\ntwo\n").expect("change a file");
    window.new_thread_on(&review, 1, 2, "two");

    let (_, sitting) = window.thread_states(&workspace);
    assert_eq!(sitting.len(), 1);
    assert_eq!(
        scratch.file_in_state(&workspace, &sitting[0], "a.txt"),
        "one\ntwo\n",
        "the thread's state holds the line it comments on"
    );
    window.stop(keeper);
}

#[test]
fn a_change_of_what_is_checked_out_is_told_with_the_list_of_workspaces_as_it_is_then() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    assert_eq!(window.show(Some(&workspace)), Ok(Reply::Done));
    scratch.wait_until_watched(&workspace);

    git(&repo, &["checkout", "--quiet", "-b", "other"]);
    let on_other = |found: &yaar_link::Workspace| {
        found.id == workspace && found.head == Some(yaar_link::Head::Branch("other".to_owned()))
    };
    window.wait_for(|fact| {
        matches!(fact, Fact::Workspaces(list)
            if list.iter().flat_map(|repository| &repository.workspaces).any(on_other))
    });
    window.stop(keeper);
}

#[test]
fn writes_that_follow_each_other_are_told_once_at_the_start_and_then_once_in_each_calm_time_with_the_last_after_the_last_write()
 {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    assert_eq!(window.show(Some(&workspace)), Ok(Reply::Done));
    scratch.wait_until_watched(&workspace);
    window.facts_until_quiet();

    // Sixty writes, one due every 20 ms from the first (an agent's pace): a schedule, not a
    // sleep after each, so a slow machine catches up and the span stays near the 1.2 s the
    // bound uses.
    let writing = {
        let file = repo.join("a.txt");
        std::thread::spawn(move || {
            let start = Instant::now();
            let mut first_write = None;
            let mut last_write = start;
            for n in 0..60u32 {
                std::fs::write(&file, format!("{n}\n")).expect("write a file");
                last_write = Instant::now();
                first_write.get_or_insert(last_write);
                let due = start + Duration::from_millis(20) * (n + 1);
                std::thread::sleep(due.saturating_duration_since(Instant::now()));
            }
            (first_write.expect("a first write"), last_write)
        })
    };
    let told: Vec<Instant> = window
        .facts_until_quiet()
        .into_iter()
        .filter(|(_, fact)| changes_of(&workspace)(fact))
        .map(|(at, _)| at)
        .collect();
    let (first_write, last_write) = writing.join().expect("the writes");

    // At most one fact for the start, one per calm time in the span (rounded up), one for the rest.
    // The bound must stay at half the writes or less, or a Keeper that told most writes passes.
    let span = last_write - first_write;
    let calm_times = span.as_millis().div_ceil(250) as usize;
    let most = calm_times + 2;
    assert!(
        most * 2 <= 60,
        "the sixty writes spread over {span:?} leave room for {most} facts, more than half \
         of them: too slow a run to show that the facts are fewer than the writes"
    );
    assert!(
        (2..=most).contains(&told.len()),
        "{} facts for sixty writes spread over {span:?}, where at most {most} are told: \
         the start, one in each of {calm_times} calm times of 250 ms, and the last; \
         one for each write would be 60",
        told.len()
    );
    for pair in told.windows(2) {
        let gap = pair[1] - pair[0];
        assert!(
            gap >= Duration::from_millis(150),
            "two facts {gap:?} apart, with a calm time of 250 ms"
        );
    }
    assert!(
        told.last().is_some_and(|last| *last >= last_write),
        "no fact came after the last write"
    );
    window.stop(keeper);
}

#[test]
fn a_worktree_is_no_longer_watched_when_the_window_that_showed_it_goes_or_shows_another() {
    let scratch = Scratch::new();
    let one = scratch.repository("one");
    let two = scratch.repository("two");
    let three = scratch.repository("three");
    let keeper = scratch.keeper();
    let mut shows_two = scratch.window();
    let first = shows_two.open(&one);
    let second = shows_two.open(&two);
    let third = shows_two.open(&three);
    assert_eq!(shows_two.show(Some(&second)), Ok(Reply::Done));
    scratch.wait_until_watched(&second);

    let mut goes = scratch.window();
    assert_eq!(goes.show(Some(&first)), Ok(Reply::Done));
    let mut moves = scratch.window();
    assert_eq!(moves.show(Some(&third)), Ok(Reply::Done));
    scratch.wait_until_watched(&first);
    scratch.wait_until_watched(&third);
    drop(goes);
    assert_eq!(moves.show(Some(&second)), Ok(Reply::Done));
    shows_two.facts_until_quiet();

    std::fs::write(one.join("a.txt"), "one\nchanged\n").expect("change a file");
    std::fs::write(three.join("a.txt"), "one\nchanged\n").expect("change a file");
    std::fs::write(two.join("a.txt"), "one\nchanged\n").expect("change a file");
    let told = shows_two.facts_until_quiet();
    assert!(
        told.iter().any(|(_, fact)| changes_of(&second)(fact)),
        "the change of the worktree that is shown was not told: {told:?}"
    );
    for (_, fact) in &told {
        assert!(
            !changes_of(&first)(fact) && !changes_of(&third)(fact),
            "a worktree that nothing shows was told: {fact:?}"
        );
    }
    shows_two.stop(keeper);
}

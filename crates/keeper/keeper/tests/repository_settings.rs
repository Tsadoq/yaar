//! Component tests: the user's answer on the language servers inside a repository, as a
//! Window sets and reads it, with a real repository, a real database and the Keeper as the
//! library on a thread of the test.
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
    Fact, FlowForm, Hello, Message, Refusal, Reply, Repository, RepositoryId, Request, RequestId,
    Setting, WorkspaceId, handshake, read_message, write_message,
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
            .join(format!(
                "yaar-keeper-repository-settings-{}-{n}",
                std::process::id()
            ));
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

    /// Attaches as a Window does, waiting for the Keeper to listen, and waiting until it
    /// serves this Window: a fact told before that is told to a Window that is not there
    /// yet, and the first reply is the proof that it is.
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
                let mut window = Window {
                    stream,
                    next_id: 0,
                    facts: Vec::new(),
                    looked_at: 0,
                    replies: Vec::new(),
                };
                // The Keeper tells a fact to the Windows it serves: its first reply comes
                // after the Window is one of them. `ListClis` tells no fact and keeps no
                // state.
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

    /// A repository in the folder `name` with one commit on `main`, and an identity of its
    /// own, since git guesses none on every system.
    fn repository(&self, name: &str) -> PathBuf {
        let folder = self.0.join(name);
        std::fs::create_dir_all(&folder).expect("make the folder");
        git(&folder, &["init", "--quiet", "-b", "main"]);
        git(&folder, &["config", "user.name", "A Test"]);
        git(&folder, &["config", "user.email", "test@example.invalid"]);
        std::fs::write(folder.join("notes.txt"), "notes\n").expect("write a file");
        git(&folder, &["add", "--all"]);
        git(&folder, &["commit", "--quiet", "-m", "first"]);
        folder
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

    /// The next `SettingChanged` from here on, passing over the facts before it.
    fn setting_changed(&mut self) -> Setting {
        loop {
            while self.looked_at < self.facts.len() {
                self.looked_at += 1;
                if let Fact::SettingChanged(setting) = &self.facts[self.looked_at - 1] {
                    return setting.clone();
                }
            }
            self.read("a setting changed");
        }
    }

    /// How many `SettingChanged` this Window was told so far.
    fn told(&self) -> usize {
        self.facts
            .iter()
            .filter(|fact| matches!(fact, Fact::SettingChanged(_)))
            .count()
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

    /// The repositories the Keeper lists, with their worktrees.
    fn repositories(&mut self) -> Vec<Repository> {
        match self.ask(Request::ListWorkspaces) {
            Ok(Reply::Workspaces(repositories)) => repositories,
            other => panic!("no repositories: {other:?}"),
        }
    }

    /// The repository that holds `workspace`, as the Keeper names it.
    fn repository_of(&mut self, workspace: &WorkspaceId) -> RepositoryId {
        self.repositories()
            .into_iter()
            .find(|repository| repository.workspaces.iter().any(|w| w.id == *workspace))
            .unwrap_or_else(|| panic!("no repository holds {workspace}"))
            .id
    }

    fn settings(&mut self) -> Vec<Setting> {
        match self.ask(Request::GetSettings) {
            Ok(Reply::Settings(settings)) => settings,
            other => panic!("no settings: {other:?}"),
        }
    }

    /// Sets `setting` and expects the Keeper to have kept it.
    fn set(&mut self, setting: &Setting) {
        let done = self.ask(Request::SetSetting {
            setting: setting.clone(),
        });
        assert!(matches!(done, Ok(Reply::Done)), "{done:?}");
    }

    fn stop(mut self, keeper: JoinHandle<std::io::Result<Exit>>) {
        write_message(&mut self.stream, &Message::Shutdown).expect("ask for the stop");
        assert_eq!(keeper.join().expect("join").expect("run"), Exit::Shutdown);
    }
}

fn answer(repository: &RepositoryId, allowed: bool) -> Setting {
    Setting::ServersInRepository {
        repository: repository.clone(),
        allowed,
    }
}

#[test]
fn an_answer_on_a_repository_is_kept_told_to_every_window_after_the_reply_and_listed() {
    let scratch = Scratch::new();
    let repo = scratch.repository("a");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    let repository = window.repository_of(&workspace);
    let mut second = scratch.window();
    assert_eq!(window.settings(), []);

    let yes = answer(&repository, true);
    let (reply, facts_before) = window.ask_noting_facts(Request::SetSetting {
        setting: yes.clone(),
    });
    assert!(matches!(reply, Ok(Reply::Done)), "{reply:?}");
    // Every Window is told, the one that set it after its reply.
    assert_eq!(window.setting_changed(), yes);
    assert_eq!(second.setting_changed(), yes);
    let told_at = window
        .facts
        .iter()
        .position(|fact| matches!(fact, Fact::SettingChanged(_)))
        .expect("the fact");
    assert!(
        told_at >= facts_before,
        "the fact came before the reply it follows"
    );
    assert_eq!(window.settings(), std::slice::from_ref(&yes));
    assert_eq!(second.settings(), [yes]);

    // A second answer takes the place of the first: one setting, the new value, told again.
    let no = answer(&repository, false);
    second.set(&no);
    assert_eq!(window.setting_changed(), no);
    assert_eq!(second.setting_changed(), no);
    assert_eq!(window.settings(), std::slice::from_ref(&no));
    assert_eq!(second.settings(), [no]);
    window.stop(keeper);
}

#[test]
fn the_answers_are_listed_by_a_keeper_started_again_on_the_same_data_folder() {
    let scratch = Scratch::new();
    let (a, b) = (scratch.repository("a"), scratch.repository("b"));
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let first = window.open(&a);
    let other = window.open(&b);
    let (first, other) = (window.repository_of(&first), window.repository_of(&other));
    assert_ne!(first, other);
    // Set in the opposite order to the one the repositories were opened in; a change
    // of the first one's answer is the last.
    window.set(&answer(&other, false));
    window.set(&answer(&first, false));
    window.set(&answer(&first, true));
    window.stop(keeper);

    let keeper = scratch.keeper();
    let mut window = scratch.window();
    assert_eq!(
        window.settings(),
        [answer(&first, true), answer(&other, false)]
    );
    window.stop(keeper);
}

#[test]
fn an_answer_on_a_repository_the_keeper_does_not_know_is_refused_naming_it_and_tells_nobody() {
    let scratch = Scratch::new();
    let repo = scratch.repository("a");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let mut second = scratch.window();
    let workspace = window.open(&repo);
    let repository = window.repository_of(&workspace);

    let unknown = RepositoryId::new("0123456789abcdef0123456789abcdef");
    let refused = window.ask(Request::SetSetting {
        setting: answer(&unknown, true),
    });
    assert!(
        matches!(&refused, Err(Refusal::Failed { reason }) if reason.contains(unknown.as_str())),
        "{refused:?}"
    );
    // The refusal itself is the barrier for what is kept: a list asked after it holds
    // nothing.
    assert_eq!(window.settings(), []);

    // The barrier for what is told: an answer that is kept is told after the refused one
    // would have been, to the same Windows, so that nothing told about the first can still
    // be on its way when it is read.
    let kept = answer(&repository, true);
    window.set(&kept);
    assert_eq!(second.setting_changed(), kept);
    assert_eq!(window.setting_changed(), kept);
    assert_eq!((window.told(), second.told()), (1, 1));
    assert_eq!(window.settings(), [kept]);
    window.stop(keeper);
}

#[test]
fn one_answer_serves_every_worktree_of_a_repository_and_not_another_repository() {
    let scratch = Scratch::new();
    let main = scratch.repository("a");
    let linked = scratch.0.join("a-linked");
    git(
        &main,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "second",
            linked.to_str().expect("a path of text"),
        ],
    );
    let apart = scratch.repository("b");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let in_main = window.open(&main);
    let in_linked = window.open(&linked);
    let in_apart = window.open(&apart);
    assert_ne!(in_main, in_linked);

    // The Keeper names the two worktrees of one repository by one id, and the other
    // repository by its own.
    let repository = window.repository_of(&in_main);
    assert_eq!(window.repository_of(&in_linked), repository);
    let apart = window.repository_of(&in_apart);
    assert_ne!(apart, repository);

    // Answered once, through either worktree's repository id, it is one setting, and
    // after a restart it is still one.
    let mut second = scratch.window();
    let no = answer(&repository, false);
    window.set(&no);
    assert_eq!(second.setting_changed(), no);
    assert_eq!(window.settings(), std::slice::from_ref(&no));
    window.stop(keeper);

    let keeper = scratch.keeper();
    let mut window = scratch.window();
    assert_eq!(window.settings(), std::slice::from_ref(&no));
    // The other repository has no answer, and a form of Flow set in a worktree is listed
    // before the answers.
    let form = Setting::FlowForm {
        workspace: in_linked,
        form: FlowForm::LiveDiff,
    };
    window.set(&form);
    window.set(&answer(&apart, true));
    assert_eq!(window.settings(), [form, no, answer(&apart, true)]);
    window.stop(keeper);
}

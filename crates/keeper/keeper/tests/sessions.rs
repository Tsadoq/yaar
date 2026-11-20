//! Component tests: the sessions of the agents' side of the Keeper and what each is told of
//! its review, through the surface a Window uses and the built hook program, with real
//! repositories, a real database and real shells beneath.
//!
//! A session exists because the hook program reported it, as Claude Code's does; the
//! Keeper is the library on a thread of the test. The Keeper runs git and the shells of
//! its terminals with the environment of its process, so each test sets that environment
//! as its first statement, and these tests are run by `cargo nextest`, which gives each
//! test a process of its own.

use std::io::Write as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_keeper::{Exit, Options, run};
use yaar_link::{
    AgentReport, Anchor, Attachment, CommentId, DraftOn, Fact, Hello, Lines, Message, Program,
    Refusal, Reply, Request, RequestId, ReviewId, SOCKET_VARIABLE, SessionState, Side, Step,
    TERMINAL_VARIABLE, TerminalId, TerminalSize, ThreadId, WorkspaceAgents, WorkspaceId,
    WorkspaceReviews, handshake, read_message, write_message,
};
use yaar_recorder::Git;
use yaar_review_engine::Reviews;
use yaar_store::Store;
use yaar_system_tests::program;
use yaar_system_tests::recorded::{self, HookRecord};

/// How long a test waits for the Keeper before it fails.
const DEADLINE: Duration = Duration::from_secs(20);

const SIZE: TerminalSize = TerminalSize {
    columns: 80,
    rows: 24,
};

/// The recording whose first session starts (line 0) and ends (line 13), and whose second
/// session starts (line 14).
const DIALOGS: &str = "claude/permission-dialogs.jsonl";

/// The recording of a session with tools: it starts (line 1), its user types a prompt
/// (2), it reads (4), edits (10) and writes (13), stops (32); the CLI starts a turn of its
/// own (33), which stops (34); it ends (35).
const TOOLS: &str = "claude/turn-tools.jsonl";

/// The recording of a session whose user types a prompt (line 2) after it starts (0).
const TYPED: &str = "claude/review-file-read.jsonl";

/// A session that starts (line 0) and nothing else of use here.
const AT_MENTION: &str = "claude/at-mention.jsonl";

/// The recording of pi's session with its four tools: its first session starts (line 1,
/// `session_start`, its file `.../e7/2026-05-23T19-20-32-010Z_0000001f-....jsonl`) and
/// ends (127); a second one starts after it (129).
const PI_TOOLS: &str = "pi/turn-tools.jsonl";

/// What `a.txt` of every repository holds, a line each.
const LINES: [&str; 3] = ["one", "two", "three"];

/// A folder of the test's own with a short path, removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    /// Makes the folder and sets the environment of this process for git and for the
    /// shell of a terminal: no configuration of the user or of the system, a fixed
    /// author, the shell `/bin/sh`, an empty home. It has to be the first statement of a
    /// test.
    fn new() -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        // `/tmp` is a link on macOS, and the Keeper names a worktree by its resolved path.
        let root = Path::new("/tmp")
            .canonicalize()
            .expect("the temporary folder")
            .join(format!("yaar-keeper-sessions-{}-{n}", std::process::id()));
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
            // Where the Keeper's copies of git's lists of files go, and a program of a
            // terminal's too: a folder that goes with the test.
            std::env::set_var("TMPDIR", root.join("tmp"));
            for name in [
                "GIT_DIR",
                "GIT_WORK_TREE",
                "GIT_INDEX_FILE",
                "GIT_CONFIG_COUNT",
                "GIT_CONFIG_PARAMETERS",
                "GIT_CONFIG",
                "ENV",
                "PS1",
            ] {
                std::env::remove_var(name);
            }
        }
        Scratch(root)
    }

    /// [`Scratch::new`], and the user's shell a program of the test's own: it writes down
    /// the arguments it is run with, as a launcher gives them, and runs no CLI, then reads
    /// its terminal until the Keeper ends it. It has to be the first statement of a test.
    fn with_launcher_shell() -> Scratch {
        let scratch = Scratch::new();
        let shell = scratch.0.join("shell");
        std::fs::write(
            &shell,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nexec cat > /dev/null\n",
                scratch.shell_log().display()
            ),
        )
        .expect("write the shell");
        let mut mode = std::fs::metadata(&shell).expect("the shell").permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut mode, 0o755);
        std::fs::set_permissions(&shell, mode).expect("make it run");
        // SAFETY: as in `Scratch::new`: this is the first statement of a test run by
        // `cargo nextest`, in a process of its own that has started no thread.
        unsafe { std::env::set_var("SHELL", &shell) };
        scratch
    }

    /// Where the shell of [`Scratch::with_launcher_shell`] writes its arguments.
    fn shell_log(&self) -> PathBuf {
        self.0.join("shell-arguments")
    }

    /// The arguments the shell of the test was run with, once it was: a launcher gives
    /// `-l`, `-i`, `-c` and the command.
    fn launched_with(&self) -> Vec<String> {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let written = std::fs::read_to_string(self.shell_log()).unwrap_or_default();
            let lines: Vec<String> = written.lines().map(str::to_owned).collect();
            if lines.len() == 4 {
                return lines;
            }
            assert!(
                Instant::now() < deadline,
                "the shell never ran: {written:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn socket(&self) -> PathBuf {
        self.0.join("k.sock")
    }

    fn database(&self) -> PathBuf {
        self.0.join("data").join(yaar_store::FILE_NAME)
    }

    /// The folder of the test's own copy of the built hook program, made at the first
    /// call. Every test process asks `program` to build, and cargo puts a program it built
    /// in place again each time: where tests run side by side, the program is gone for a
    /// moment, and one started from there fails. A copy is made at the first try that finds
    /// the program, and is the test's own.
    fn companions(&self) -> PathBuf {
        let folder = self.0.join("companions");
        let copy = folder.join(yaar_adapter_claude::HOOK_PROGRAM);
        if copy.is_file() {
            return folder;
        }
        std::fs::create_dir_all(&folder).expect("make the folder");
        let deadline = Instant::now() + DEADLINE;
        loop {
            let built = std::panic::catch_unwind(|| program("yaar-claude-hook"));
            if let Ok(built) = built
                && std::fs::copy(built, &copy).is_ok()
            {
                return folder;
            }
            assert!(Instant::now() < deadline, "the hook program is never there");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Starts the Keeper's library on a thread of this test, with the folder of the hook
    /// program as where the pieces of the agents lie.
    fn keeper(&self) -> JoinHandle<std::io::Result<Exit>> {
        self.keeper_told(Some(self.companions()))
    }

    /// The same, told this folder for the pieces, or none.
    fn keeper_told(&self, companions: Option<PathBuf>) -> JoinHandle<std::io::Result<Exit>> {
        let options = Options {
            socket: self.socket(),
            linger: Duration::from_secs(60),
            app_version: "test-keeper".to_owned(),
            data_dir: self.0.join("data"),
            companions,
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
                return Window::new(stream);
            }
            assert!(
                Instant::now() < deadline,
                "no Keeper at {:?}",
                self.socket()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// A repository in the folder `name` with one commit on `main` and an identity of its
    /// own, since git guesses none on every system.
    fn repository(&self, name: &str) -> PathBuf {
        let folder = self.0.join(name);
        std::fs::create_dir_all(&folder).expect("make the folder");
        git(&folder, &["init", "--quiet", "-b", "main"]);
        git(&folder, &["config", "user.name", "A Test"]);
        git(&folder, &["config", "user.email", "test@example.invalid"]);
        std::fs::write(folder.join("a.txt"), "one\ntwo\nthree\n").expect("write a file");
        git(&folder, &["add", "--all"]);
        git(&folder, &["commit", "--quiet", "-m", "first"]);
        folder
    }

    /// The database the Keeper kept, read once the Keeper has stopped.
    fn store(&self) -> Store {
        Store::open(&self.database()).expect("open the Keeper's database")
    }

    /// Runs the hook program with `payload` on its standard input, as Claude Code does,
    /// in the terminal `terminal` of the app; none for a CLI outside the app. It must
    /// succeed; what it prints is what the Keeper answered, which no test here reads.
    fn hook(&self, terminal: Option<&TerminalId>, payload: &[u8]) {
        let mut command = Command::new(self.companions().join(yaar_adapter_claude::HOOK_PROGRAM));
        command
            .env(SOCKET_VARIABLE, self.socket())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        match terminal {
            Some(terminal) => command.env(TERMINAL_VARIABLE, terminal.as_str()),
            None => command.env_remove(TERMINAL_VARIABLE),
        };
        let mut child = command.spawn().expect("start the hook program");
        child
            .stdin
            .take()
            .expect("its standard input")
            .write_all(payload)
            .expect("hand it the hook's input");
        let status = child.wait().expect("wait for the hook program");
        assert!(status.success(), "the hook program failed");
    }
}

impl Scratch {
    /// Reports `payload` as the extension of pi would, on a connection of its own as the
    /// reporting program's is, for the CLI process `pid` in no terminal of the app: the
    /// folder of the payload says where it belongs. The Keeper answers it `Done`.
    fn pi_report(&self, pid: u32, payload: Vec<u8>) {
        let report = AgentReport {
            agent: yaar_adapter_pi::NAME.to_owned(),
            terminal: None,
            pid,
            payload,
        };
        assert_eq!(
            self.window().ask(Request::Report { report }),
            Ok(Reply::Done)
        );
    }
}

/// A process the test started, killed and collected when the test ends, whatever way it
/// does.
struct Process(std::process::Child);

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
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
    replies: Vec<(RequestId, Result<Reply, Refusal>)>,
    output: Vec<(TerminalId, Vec<u8>)>,
}

impl Window {
    fn new(stream: UnixStream) -> Window {
        Window {
            stream,
            next_id: 0,
            facts: Vec::new(),
            looked_at: 0,
            replies: Vec::new(),
            output: Vec::new(),
        }
    }

    fn send(&mut self, request: Request) -> RequestId {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        write_message(&mut self.stream, &Message::Request { id, request }).expect("send");
        id
    }

    /// Reads one frame and keeps it. Fails the test at the deadline, with the facts that
    /// came.
    fn read(&mut self, waiting_for: &str) {
        match read_message(&mut self.stream) {
            Ok(Message::Fact(fact)) => self.facts.push(fact),
            Ok(Message::Reply { to, reply }) => self.replies.push((to, reply)),
            Ok(Message::Output { terminal, bytes }) => {
                match self.output.iter_mut().find(|(from, _)| *from == terminal) {
                    Some((_, heard)) => heard.extend(bytes),
                    None => self.output.push((terminal, bytes)),
                }
            }
            Ok(other) => panic!("waiting for {waiting_for}, a frame nobody sends: {other:?}"),
            Err(e) => panic!(
                "waiting for {waiting_for}: no frame from the Keeper ({e}); the facts so far: {:#?}",
                self.facts
            ),
        }
    }

    /// Sends a request and waits for its reply.
    fn ask(&mut self, request: Request) -> Result<Reply, Refusal> {
        let id = self.send(request);
        loop {
            if let Some(at) = self.replies.iter().position(|(to, _)| *to == id) {
                return self.replies.remove(at).1;
            }
            self.read("a reply");
        }
    }

    /// The first fact from here on that `wanted` takes, passing over the ones before it.
    fn until<T>(&mut self, what: &str, mut wanted: impl FnMut(&Fact) -> Option<T>) -> T {
        loop {
            while self.looked_at < self.facts.len() {
                self.looked_at += 1;
                if let Some(found) = wanted(&self.facts[self.looked_at - 1]) {
                    return found;
                }
            }
            self.read(what);
        }
    }

    /// The next `AgentsChanged` of `workspace` that `wanted` takes.
    fn agents_until(
        &mut self,
        workspace: &WorkspaceId,
        what: &str,
        wanted: impl Fn(&WorkspaceAgents) -> bool,
    ) -> WorkspaceAgents {
        self.until(what, |fact| match fact {
            Fact::AgentsChanged(agents) if agents.workspace == *workspace && wanted(agents) => {
                Some(agents.clone())
            }
            _ => None,
        })
    }

    /// Reads until `text` is in the output of `terminal`.
    fn output_until(&mut self, terminal: &TerminalId, text: &str) {
        loop {
            let heard = self.output.iter().find(|(from, _)| from == terminal);
            if heard.is_some_and(|(_, bytes)| String::from_utf8_lossy(bytes).contains(text)) {
                return;
            }
            self.read(text);
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

    /// Opens a terminal with a shell in it, and waits until the shell has started.
    fn open_terminal(&mut self, workspace: &WorkspaceId) -> TerminalId {
        let opened = self.ask(Request::OpenTerminal {
            workspace: workspace.clone(),
            program: Program::Shell,
            size: SIZE,
        });
        let Ok(Reply::TerminalOpened { terminal }) = opened else {
            panic!("no terminal: {opened:?}");
        };
        // A line the typed one does not hold: the shell is reading when it is seen.
        self.type_into(&terminal, b"printf 'ready-%s\\n' now\r");
        self.output_until(&terminal, "ready-now");
        terminal
    }

    /// Opens a terminal that runs `program` and gives its id. It is not waited for: what
    /// the test reads says when its program ran.
    fn open_program(&mut self, workspace: &WorkspaceId, program: Program) -> TerminalId {
        let opened = self.ask(Request::OpenTerminal {
            workspace: workspace.clone(),
            program,
            size: SIZE,
        });
        let Ok(Reply::TerminalOpened { terminal }) = opened else {
            panic!("no terminal: {opened:?}");
        };
        terminal
    }

    fn type_into(&mut self, terminal: &TerminalId, bytes: &[u8]) {
        write_message(
            &mut self.stream,
            &Message::Input {
                terminal: terminal.clone(),
                bytes: bytes.to_vec(),
            },
        )
        .expect("send the bytes");
    }

    fn agents(&mut self) -> Vec<WorkspaceAgents> {
        match self.ask(Request::ListAgents) {
            Ok(Reply::Agents(agents)) => agents,
            other => panic!("no list of agents: {other:?}"),
        }
    }

    fn stop(mut self, keeper: JoinHandle<std::io::Result<Exit>>) {
        write_message(&mut self.stream, &Message::Shutdown).expect("ask for the stop");
        assert_eq!(keeper.join().expect("join").expect("run"), Exit::Shutdown);
    }
}

impl Window {
    /// The reviews of a workspace.
    fn reviews(&mut self, workspace: &WorkspaceId) -> WorkspaceReviews {
        match self.ask(Request::GetReviews {
            workspace: workspace.clone(),
        }) {
            Ok(Reply::Reviews(reviews)) => reviews,
            other => panic!("no reviews: {other:?}"),
        }
    }

    /// Saves the first comment of a new thread on line `line` (from 1) of `a.txt`.
    fn draft(&mut self, review: &ReviewId, n: u32, line: u32, text: &str) -> (ThreadId, CommentId) {
        let (thread, comment) = ids(n);
        let saved = self.ask(Request::SaveDraft {
            review: review.clone(),
            thread: thread.clone(),
            comment: comment.clone(),
            on: DraftOn::NewThread {
                anchor: Anchor {
                    tree: None,
                    path: PathBuf::from("a.txt"),
                    side: Side::New,
                    lines: Lines {
                        first: line,
                        last: line,
                    },
                    text: LINES[line as usize - 1].to_owned(),
                },
            },
            text: text.to_owned(),
        });
        assert_eq!(saved, Ok(Reply::Done));
        (thread, comment)
    }

    /// The facts that came after the first `mark` ones, ReviewsChanged of `workspace`
    /// counted.
    fn reviews_changed_since(&self, mark: usize, workspace: &WorkspaceId) -> usize {
        self.facts[mark..]
            .iter()
            .filter(|fact| matches!(fact, Fact::ReviewsChanged { workspace: told } if told == workspace))
            .count()
    }
}

/// The id of the thread `n` and of its first comment, 32 hexadecimal digits each.
fn ids(n: u32) -> (ThreadId, CommentId) {
    (
        ThreadId::new(format!("{:032x}", 0x1000 + n)),
        CommentId::new(format!("{:032x}", 0x2000 + n)),
    )
}

/// A Keeper with one repository open and a Window attached.
struct Setup {
    scratch: Scratch,
    keeper: JoinHandle<std::io::Result<Exit>>,
    window: Window,
    repo: PathBuf,
    workspace: WorkspaceId,
}

impl Setup {
    /// Has to be the first statement of a test.
    fn new() -> Setup {
        let scratch = Scratch::new();
        let repo = scratch.repository("repo");
        let keeper = scratch.keeper();
        let mut window = scratch.window();
        let workspace = window.open(&repo);
        Setup {
            scratch,
            keeper,
            window,
            repo,
            workspace,
        }
    }

    /// Runs the hook of line `line` of `records`, moved into the repository, in `terminal`.
    fn hook_in(&self, terminal: &TerminalId, records: &[HookRecord], line: usize) {
        let record = records[line].in_project(&self.repo);
        self.scratch.hook(Some(terminal), &record.input_bytes());
    }

    /// A session of Claude Code that runs in a shell of the app, and the review it holds:
    /// the first session of the recording of dialogs, waiting for the user's message.
    fn session(&mut self) -> (yaar_link::SessionId, ReviewId, TerminalId) {
        let terminal = self.window.open_terminal(&self.workspace);
        let records = recorded::hook_events(DIALOGS);
        self.hook_in(&terminal, &records, 0);
        let started =
            self.window
                .agents_until(&self.workspace, "the session", is_state(SessionState::Idle));
        let session = &started.sessions[0];
        (
            session.id.clone(),
            session.review.clone().expect("a session holds a review"),
            terminal,
        )
    }

    fn attach(&mut self, review: &ReviewId, thread: Option<&ThreadId>) -> Result<Reply, Refusal> {
        self.window.ask(Request::AttachReview {
            review: review.clone(),
            thread: thread.cloned(),
        })
    }

    /// Reads until a `Step` of the workspace that `wanted` takes is told.
    fn step_told(&mut self, what: &str, wanted: impl Fn(&Step) -> bool) {
        let workspace = self.workspace.clone();
        self.window.until(what, |fact| match fact {
            Fact::Step {
                workspace: told,
                step,
                ..
            } if *told == workspace && wanted(step) => Some(()),
            _ => None,
        });
    }

    /// The session list of the workspace, as a Window reads it.
    fn sessions(&mut self) -> Vec<yaar_link::Session> {
        self.window
            .agents()
            .into_iter()
            .find(|agents| agents.workspace == self.workspace)
            .map(|agents| agents.sessions)
            .unwrap_or_default()
    }

    fn review(&mut self, review: &ReviewId) -> yaar_link::Review {
        self.window
            .reviews(&self.workspace)
            .reviews
            .into_iter()
            .find(|held| held.id == *review)
            .expect("the review is kept")
    }

    /// Stops the Keeper and gives what it kept, once it has gone.
    fn finish(self) -> (Store, Scratch) {
        self.window.stop(self.keeper);
        let store = self.scratch.store();
        (store, self.scratch)
    }
}

fn is_state(state: SessionState) -> impl Fn(&WorkspaceAgents) -> bool {
    move |agents| agents.sessions.iter().any(|session| session.state == state)
}

/// The head commit of the repository in `folder`.
fn head(folder: &Path) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(folder)
        .args(["rev-parse", "HEAD"])
        .stdin(Stdio::null())
        .output()
        .expect("start git");
    assert!(output.status.success(), "git rev-parse failed");
    String::from_utf8(output.stdout)
        .expect("a commit id")
        .trim()
        .to_owned()
}

/// The path of the file the tool of line `line` of `records` names, relative to the
/// project the recording was made in.
fn path_of(records: &[HookRecord], line: usize) -> PathBuf {
    let file = records[line].input["tool_input"]["file_path"]
        .as_str()
        .expect("the file of the tool");
    Path::new(file)
        .strip_prefix(recorded::PROJECT)
        .expect("a file of the project")
        .to_owned()
}

#[test]
fn a_turn_a_typed_prompt_starts_is_from_the_user_and_none_runs_after_it_stops() {
    let mut setup = Setup::new();
    let terminal = setup.window.open_terminal(&setup.workspace);
    let records = recorded::hook_events(TYPED);
    setup.hook_in(&terminal, &records, 0);
    setup.hook_in(&terminal, &records, 2);
    let working = setup.window.agents_until(
        &setup.workspace,
        "the turn of the typed prompt",
        is_state(SessionState::Works),
    );
    assert!(working.sessions[0].turn.is_some());
    assert!(working.sessions[0].turn_from_user);
    assert!(setup.sessions()[0].turn_from_user, "as ListAgents tells it");

    // The turn stops: none runs, so none is from the user.
    setup.hook_in(&terminal, &records, 6);
    let stopped = setup.window.agents_until(
        &setup.workspace,
        "the turn stopped",
        is_state(SessionState::Unreviewed),
    );
    assert_eq!(stopped.sessions[0].turn, None);
    assert!(!stopped.sessions[0].turn_from_user);
    let _ = setup.finish();
}

#[test]
fn a_turn_the_cli_starts_itself_is_not_from_the_user_though_the_one_before_it_was() {
    let mut setup = Setup::new();
    let terminal = setup.window.open_terminal(&setup.workspace);
    let records = recorded::hook_events(TOOLS);
    for line in [1, 2] {
        setup.hook_in(&terminal, &records, line);
    }
    let typed = setup.window.agents_until(
        &setup.workspace,
        "the turn of the typed prompt",
        is_state(SessionState::Works),
    );
    assert!(typed.sessions[0].turn_from_user);
    setup.hook_in(&terminal, &records, 32);
    setup.window.agents_until(
        &setup.workspace,
        "the turn stopped",
        is_state(SessionState::Unreviewed),
    );

    // A background task ended, and Claude Code starts a turn that no message of the user
    // began.
    setup.hook_in(&terminal, &records, 33);
    let own = setup.window.agents_until(
        &setup.workspace,
        "the turn the CLI started",
        is_state(SessionState::Works),
    );
    assert!(own.sessions[0].turn.is_some());
    assert!(!own.sessions[0].turn_from_user);
    assert!(
        !setup.sessions()[0].turn_from_user,
        "as ListAgents tells it"
    );
    let _ = setup.finish();
}

#[test]
fn what_is_attached_of_a_sessions_review_follows_attaching_and_detaching_each_told_to_the_windows()
{
    let mut setup = Setup::new();
    let (session, review, _terminal) = setup.session();
    let (thread, _) = setup.window.draft(&review, 1, 2, "why?");
    assert_eq!(setup.sessions()[0].attached, None);

    assert!(matches!(
        setup.attach(&review, None),
        Ok(Reply::AttachedReview { .. })
    ));
    let told = setup.window.agents_until(
        &setup.workspace,
        "the session told with its attachment",
        |agents| agents.sessions.iter().any(|s| s.attached.is_some()),
    );
    assert_eq!(told.sessions[0].id, session);
    assert_eq!(told.sessions[0].attached, Some(Attachment::Review));
    assert_eq!(setup.sessions()[0].attached, Some(Attachment::Review));

    // One thread alone replaces the review.
    assert!(setup.attach(&review, Some(&thread)).is_ok());
    let alone = setup.window.agents_until(
        &setup.workspace,
        "the session told with one thread attached",
        |agents| {
            agents
                .sessions
                .iter()
                .any(|s| s.attached == Some(Attachment::Thread(thread.clone())))
        },
    );
    assert_eq!(alone.sessions.len(), 1);

    assert_eq!(
        setup.window.ask(Request::DetachReview {
            review: review.clone()
        }),
        Ok(Reply::Done)
    );
    let detached = setup.window.agents_until(
        &setup.workspace,
        "the session told with nothing attached",
        |agents| agents.sessions.iter().all(|s| s.attached.is_none()),
    );
    assert_eq!(detached.sessions[0].review, Some(review));
    assert_eq!(setup.sessions()[0].attached, None);
    let _ = setup.finish();
}

#[test]
fn a_session_that_ran_when_its_keeper_stopped_is_resumable_until_it_runs_again_and_the_resume_runs_its_clis_command()
 {
    let scratch = Scratch::with_launcher_shell();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    let records = recorded::hook_events(DIALOGS);
    // In no terminal of the app: the folder says where it belongs.
    let record = records[0].in_project(&repo);
    scratch.hook(None, &record.input_bytes());
    let started = window.agents_until(&workspace, "the session", is_state(SessionState::Idle));
    let session = started.sessions[0].id.clone();
    let review = started.sessions[0].review.clone();
    assert!(!started.sessions[0].resumable, "it runs");

    // What runs is not brought back, and nor is what the Keeper does not hold.
    for refused in [
        session.clone(),
        yaar_link::SessionId::new("no-such-session"),
    ] {
        let asked = window.ask(Request::OpenTerminal {
            workspace: workspace.clone(),
            program: Program::Resume { session: refused },
            size: SIZE,
        });
        assert!(matches!(asked, Err(Refusal::Failed { .. })), "{asked:?}");
    }

    // A second session holds a review and ends by its CLI: not running, and not one the
    // Keeper's stop cut off.
    let tools = recorded::hook_events(TOOLS);
    scratch.hook(None, &tools[1].in_project(&repo).input_bytes());
    let second = window.agents_until(&workspace, "the second session", |agents| {
        agents.sessions.len() == 2
    });
    let ended_by_cli = second
        .sessions
        .iter()
        .find(|s| s.id != session)
        .expect("the second session")
        .id
        .clone();
    // A third session starts in a terminal of the app, before the second one ends, so that it
    // holds a review of its own.
    let at_mention = recorded::hook_events(AT_MENTION);
    let terminal = window.open_program(&workspace, Program::Shell);
    // Its CLI is a process of its own, as it is when its terminal ends: the hook program
    // names the test's process for every session it reports, and that would end the others.
    let cli = Process(
        Command::new("sleep")
            .arg("600")
            .stdin(Stdio::null())
            .spawn()
            .expect("start a process"),
    );
    let report = AgentReport {
        agent: yaar_adapter_claude::NAME.to_owned(),
        terminal: Some(terminal.clone()),
        pid: cli.0.id(),
        payload: at_mention[0].in_project(&repo).input_bytes(),
    };
    // On a connection of its own, as the hook program's.
    let mut piece = scratch.window();
    assert_eq!(piece.ask(Request::Report { report }), Ok(Reply::Done));
    let third = window.agents_until(&workspace, "the third session", |agents| {
        agents.sessions.len() == 3
    });
    let closed = third
        .sessions
        .iter()
        .find(|s| s.id != session && s.id != ended_by_cli)
        .expect("the third session")
        .id
        .clone();
    scratch.hook(None, &tools[35].in_project(&repo).input_bytes());
    let ended = window.agents_until(
        &workspace,
        "the second ended",
        is_state(SessionState::NotRunning),
    );
    let by_cli = ended
        .sessions
        .iter()
        .find(|s| s.id == ended_by_cli)
        .expect("the second session");
    assert_eq!(by_cli.state, SessionState::NotRunning);
    assert!(by_cli.review.is_some());
    assert!(!by_cli.resumable, "its CLI ended it: nothing cut it off");

    // The user closes the third one's terminal (the end of the file on an empty line ends
    // the shell, which only reads it): its process is gone, which is no cut-off of the
    // Keeper's either.
    window.type_into(&terminal, b"\x04");
    let gone = window.agents_until(&workspace, "the third gone with its terminal", |agents| {
        agents
            .sessions
            .iter()
            .any(|s| s.id == closed && s.state == SessionState::NotRunning)
    });
    let by_terminal = gone
        .sessions
        .iter()
        .find(|s| s.id == closed)
        .expect("the third session");
    assert!(by_terminal.review.is_some());
    assert!(
        !by_terminal.resumable,
        "its process went with its terminal: nothing cut it off"
    );
    window.stop(keeper);

    // The Keeper starts again: the first session was cut off, the others were not.
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let listed = window.agents();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].sessions.len(), 3);
    let cut_off = listed[0]
        .sessions
        .iter()
        .find(|s| s.id == session)
        .expect("the first session");
    assert_eq!(cut_off.state, SessionState::NotRunning);
    assert!(cut_off.resumable);
    assert_eq!(cut_off.review, review);
    let ended_before = listed[0]
        .sessions
        .iter()
        .find(|s| s.id == ended_by_cli)
        .expect("the second session");
    assert_eq!(ended_before.state, SessionState::NotRunning);
    assert!(
        !ended_before.resumable,
        "it ended before the Keeper stopped"
    );
    let closed_before = listed[0]
        .sessions
        .iter()
        .find(|s| s.id == closed)
        .expect("the third session");
    assert_eq!(closed_before.state, SessionState::NotRunning);
    assert!(
        !closed_before.resumable,
        "its terminal ended before the Keeper stopped"
    );

    let resumed = session.clone();
    window.open_program(&workspace, Program::Resume { session });
    let launched = scratch.launched_with();
    assert_eq!(launched[..3], ["-l", "-i", "-c"]);
    assert_eq!(
        launched[3],
        format!("claude --resume '{}'", records[0].session())
    );

    // Its CLI reports it again: it runs, and is resumable no more.
    scratch.hook(None, &record.input_bytes());
    let running = window.agents_until(&workspace, "the session runs", is_state(SessionState::Idle));
    let runs = running
        .sessions
        .iter()
        .find(|s| s.id == resumed)
        .expect("the session that runs");
    assert_eq!(runs.state, SessionState::Idle);
    assert!(!runs.resumable);
    window.stop(keeper);
}

/// A process of its own for a CLI to name as its process in a report: it runs until the test
/// ends, so that the Keeper does not take the session for gone.
fn a_cli_process() -> Process {
    Process(
        Command::new("sleep")
            .arg("600")
            .stdin(Stdio::null())
            .spawn()
            .expect("start a process"),
    )
}

/// What pi's resume runs for a session whose last record of itself is the file
/// `record_file`: the folder of the record, and the session's own id, each word in single
/// quotes for the shell.
fn pi_resume(record_file: &str, id: &str) -> String {
    let folder = Path::new(record_file).parent().expect("a file in a folder");
    format!("pi --session-dir '{}' --session '{id}'", folder.display())
}

impl Scratch {
    /// Takes away the arguments the shell was last run with, so that what [`launched_with`]
    /// reads next is what the next launch wrote.
    ///
    /// [`launched_with`]: Scratch::launched_with
    fn forget_the_launch(&self) {
        match std::fs::remove_file(self.shell_log()) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("the log of the shell could not be taken away: {e}"),
        }
    }
}

#[test]
fn a_pi_session_that_ran_when_its_keeper_stopped_is_resumable_after_each_restart_and_its_resume_runs_pi_with_the_folder_of_its_last_record_and_its_id()
 {
    let scratch = Scratch::with_launcher_shell();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    let records = recorded::pi_events(PI_TOOLS);
    let pi = a_cli_process();

    // The session starts (line 1): it runs, and is not offered.
    let first = records[1].in_project(&repo);
    scratch.pi_report(pi.0.id(), first.payload_bytes());
    let started = window.agents_until(&workspace, "the session", is_state(SessionState::Idle));
    assert_eq!(started.sessions.len(), 1);
    assert_eq!(started.sessions[0].agent, "pi");
    assert!(!started.sessions[0].resumable, "it runs");
    let session = started.sessions[0].id.clone();

    // Its next report names another file as the session's record (line 3, the one field
    // `sessionFile` changed: pi moves a session's file when it forks it), and a later one
    // names none (line 16, the field taken out): the last one named is the one kept.
    let moved = "/moved/sessions/2026-05-23T19-21-00-000Z_0000001f.jsonl";
    let mut turn = records[3].in_project(&repo);
    turn.payload["sessionFile"] = serde_json::json!(moved);
    scratch.pi_report(pi.0.id(), turn.payload_bytes());
    window.agents_until(&workspace, "the turn", is_state(SessionState::Works));
    let mut read = records[16].in_project(&repo);
    read.payload
        .as_object_mut()
        .expect("a record is an object")
        .remove("sessionFile");
    scratch.pi_report(pi.0.id(), read.payload_bytes());
    window.stop(keeper);

    // After the restart the session is offered, and its resume is pi's own command for it:
    // the folder of the record kept and the id pi gave it.
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let listed = window.agents();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].sessions.len(), 1);
    assert_eq!(listed[0].sessions[0].id, session);
    assert_eq!(listed[0].sessions[0].state, SessionState::NotRunning);
    assert!(listed[0].sessions[0].resumable);
    scratch.forget_the_launch();
    window.open_program(
        &workspace,
        Program::Resume {
            session: session.clone(),
        },
    );
    let launched = scratch.launched_with();
    assert_eq!(launched[..3], ["-l", "-i", "-c"]);
    assert_eq!(launched[3], pi_resume(moved, records[1].session()));
    window.stop(keeper);

    // Offered again after a second restart in which nothing brought it back, and with the
    // same command.
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    assert!(window.agents()[0].sessions[0].resumable);
    scratch.forget_the_launch();
    window.open_program(&workspace, Program::Resume { session });
    assert_eq!(
        scratch.launched_with()[3],
        pi_resume(moved, records[1].session())
    );
    window.stop(keeper);
}

#[test]
fn a_session_of_each_cli_cut_off_together_is_offered_the_resume_of_its_own_cli() {
    let scratch = Scratch::with_launcher_shell();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    let claude_records = recorded::hook_events(DIALOGS);
    let pi_records = recorded::pi_events(PI_TOOLS);
    let pi = a_cli_process();

    scratch.hook(None, &claude_records[0].in_project(&repo).input_bytes());
    window.agents_until(
        &workspace,
        "the session of Claude Code",
        is_state(SessionState::Idle),
    );
    let pi_start = pi_records[1].in_project(&repo);
    scratch.pi_report(pi.0.id(), pi_start.payload_bytes());
    let both = window.agents_until(&workspace, "the session of pi too", |agents| {
        agents.sessions.len() == 2
    });
    let of = |agent: &str| {
        both.sessions
            .iter()
            .find(|s| s.agent == agent)
            .unwrap_or_else(|| panic!("a session of {agent}"))
            .id
            .clone()
    };
    let (claude, pi_session) = (of(yaar_adapter_claude::NAME), of(yaar_adapter_pi::NAME));
    window.stop(keeper);

    // Both were cut off by the one stop: each is offered, and each resume is its own CLI's.
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let listed = window.agents();
    assert_eq!(listed[0].sessions.len(), 2);
    assert!(listed[0].sessions.iter().all(|s| s.resumable));
    window.open_program(
        &workspace,
        Program::Resume {
            session: claude.clone(),
        },
    );
    assert_eq!(
        scratch.launched_with()[3],
        format!("claude --resume '{}'", claude_records[0].session())
    );
    scratch.forget_the_launch();
    window.open_program(
        &workspace,
        Program::Resume {
            session: pi_session,
        },
    );
    let record_file = pi_start.payload["sessionFile"]
        .as_str()
        .expect("the record names its file");
    assert_eq!(
        scratch.launched_with()[3],
        pi_resume(record_file, pi_records[1].session())
    );
    window.stop(keeper);
}

#[test]
fn a_resume_is_refused_while_its_terminal_runs_and_allowed_again_when_it_ends_and_never_once_the_session_reports()
 {
    let scratch = Scratch::with_launcher_shell();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    let records = recorded::pi_events(PI_TOOLS);
    let pi = a_cli_process();
    let first = records[1].in_project(&repo);
    scratch.pi_report(pi.0.id(), first.payload_bytes());
    let started = window.agents_until(&workspace, "the session", is_state(SessionState::Idle));
    let session = started.sessions[0].id.clone();
    window.stop(keeper);

    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let resume = |window: &mut Window| {
        window.ask(Request::OpenTerminal {
            workspace: workspace.clone(),
            program: Program::Resume {
                session: session.clone(),
            },
            size: SIZE,
        })
    };
    let Ok(Reply::TerminalOpened { terminal }) = resume(&mut window) else {
        panic!("the resume was refused");
    };
    // The session was reported once, by its start (line 1), and the Keeper was stopped: the
    // record its resume carries is the one that start named, in the folder pi kept it in.
    let kept_by_the_start = first.payload["sessionFile"]
        .as_str()
        .expect("the start names the session's file");
    assert_eq!(
        scratch.launched_with()[3],
        pi_resume(kept_by_the_start, records[1].session())
    );

    // While its terminal runs the session is not brought back a second time. It is still
    // offered, since nothing of it was reported. The list read after the refusal, on the
    // same connection, holds the one terminal.
    let refused = resume(&mut window);
    assert!(
        matches!(refused, Err(Refusal::Failed { .. })),
        "{refused:?}"
    );
    let Ok(Reply::Terminals(terminals)) = window.ask(Request::ListTerminals) else {
        panic!("no list of terminals");
    };
    assert_eq!(terminals.len(), 1, "{terminals:?}");
    assert!(window.agents()[0].sessions[0].resumable);

    // The terminal ends with its program: the session may be brought back again. The Keeper
    // queued that end for the agents' side before it told the Windows, so a report of another
    // session of pi, on the same queue and answered once it is worked out, is the barrier:
    // what the end changes is changed when it is answered.
    let another = a_cli_process();
    window.type_into(&terminal, b"\x04");
    window.until("the end of the resume's terminal", |fact| match fact {
        Fact::TerminalEnded {
            terminal: ended, ..
        } if *ended == terminal => Some(()),
        _ => None,
    });
    scratch.pi_report(
        another.0.id(),
        records[129].in_project(&repo).payload_bytes(),
    );
    scratch.forget_the_launch();
    let Ok(Reply::TerminalOpened { terminal: second }) = resume(&mut window) else {
        panic!("the resume after its terminal ended was refused");
    };
    assert_ne!(second, terminal);
    assert_eq!(
        scratch.launched_with()[3],
        pi_resume(kept_by_the_start, records[1].session())
    );

    // The session reports: its CLI has it, and it is offered no more, though the terminal
    // of the resume still runs.
    scratch.pi_report(pi.0.id(), first.payload_bytes());
    // Picked by its id: the other session's report that served as a barrier also lists one
    // that runs and is not offered.
    window.agents_until(&workspace, "the session runs", |agents| {
        agents
            .sessions
            .iter()
            .any(|s| s.id == session && s.state == SessionState::Idle && !s.resumable)
    });

    // The resume's terminal ends, and with it the hold: only the report keeps the session
    // from being brought back now. The report of the other session is the barrier again.
    window.type_into(&second, b"\x04");
    window.until(
        "the end of the second resume's terminal",
        |fact| match fact {
            Fact::TerminalEnded {
                terminal: ended, ..
            } if *ended == second => Some(()),
            _ => None,
        },
    );
    scratch.pi_report(
        another.0.id(),
        records[129].in_project(&repo).payload_bytes(),
    );
    let refused = resume(&mut window);
    assert!(
        matches!(refused, Err(Refusal::Failed { .. })),
        "{refused:?}"
    );
    let Ok(Reply::Terminals(terminals)) = window.ask(Request::ListTerminals) else {
        panic!("no list of terminals");
    };
    assert!(terminals.is_empty(), "{terminals:?}");
    window.stop(keeper);
}

#[test]
fn the_end_of_another_terminal_does_not_free_a_session_a_resume_terminal_is_open_for() {
    let scratch = Scratch::with_launcher_shell();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    let records = recorded::pi_events(PI_TOOLS);
    let pi = a_cli_process();
    scratch.pi_report(pi.0.id(), records[1].in_project(&repo).payload_bytes());
    let started = window.agents_until(&workspace, "the session", is_state(SessionState::Idle));
    let session = started.sessions[0].id.clone();
    window.stop(keeper);

    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let resume = |window: &mut Window| {
        window.ask(Request::OpenTerminal {
            workspace: workspace.clone(),
            program: Program::Resume {
                session: session.clone(),
            },
            size: SIZE,
        })
    };
    assert!(matches!(
        resume(&mut window),
        Ok(Reply::TerminalOpened { .. })
    ));
    // Another terminal opens and ends while the resume's runs.
    let other = window.open_program(
        &workspace,
        Program::Agent {
            agent: yaar_adapter_claude::NAME.to_owned(),
        },
    );
    window.type_into(&other, b"\x04");
    window.until("the end of the other terminal", |fact| match fact {
        Fact::TerminalEnded { terminal, .. } if *terminal == other => Some(()),
        _ => None,
    });
    // The Keeper queued that end for the agents' side before it told the Windows. A report
    // of another session of pi, on the same queue and answered once it is worked out, is
    // the barrier: what the end changes is changed when it is answered.
    scratch.pi_report(pi.0.id(), records[129].in_project(&repo).payload_bytes());

    let refused = resume(&mut window);
    assert!(
        matches!(refused, Err(Refusal::Failed { .. })),
        "{refused:?}"
    );
    let Ok(Reply::Terminals(terminals)) = window.ask(Request::ListTerminals) else {
        panic!("no list of terminals");
    };
    assert_eq!(terminals.len(), 1, "only the resume's terminal runs");
    window.stop(keeper);
}

#[test]
fn a_session_still_offered_stays_listed_when_a_new_session_of_its_cli_starts_in_the_workspace_and_does_not_decide_its_state()
 {
    let scratch = Scratch::with_launcher_shell();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    let records = recorded::pi_events(PI_TOOLS);
    let pi = a_cli_process();
    scratch.pi_report(pi.0.id(), records[1].in_project(&repo).payload_bytes());
    let started = window.agents_until(&workspace, "the session", is_state(SessionState::Idle));
    let session = started.sessions[0].id.clone();
    window.stop(keeper);

    // After the restart another session of pi starts in the workspace and takes over the
    // review the first held. The first is still offered, so the Window must still be told of
    // it: its resume control is drawn from this list.
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let another = a_cli_process();
    scratch.pi_report(
        another.0.id(),
        records[129].in_project(&repo).payload_bytes(),
    );
    let both = window.agents_until(&workspace, "the new session", |agents| {
        agents
            .sessions
            .iter()
            .any(|s| s.id != session && s.state == SessionState::Idle)
    });
    let offered = both
        .sessions
        .iter()
        .find(|s| s.id == session)
        .expect("the session still offered is listed");
    assert_eq!(offered.state, SessionState::NotRunning);
    assert!(offered.resumable);
    // The session that was cut off is listed only because it can be brought back: the
    // state the workspace shows is its new session's.
    assert_eq!(both.shown, Some(SessionState::Idle));
    window.stop(keeper);
}

#[test]
fn a_pi_session_its_cli_ended_is_not_resumable_and_its_resume_is_refused_with_the_reason_before_and_after_a_restart()
 {
    let scratch = Scratch::with_launcher_shell();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    let records = recorded::pi_events(PI_TOOLS);
    let pi = a_cli_process();
    scratch.pi_report(pi.0.id(), records[1].in_project(&repo).payload_bytes());
    let started = window.agents_until(&workspace, "the session", is_state(SessionState::Idle));
    let session = started.sessions[0].id.clone();
    // Its CLI ends it (line 127).
    scratch.pi_report(pi.0.id(), records[127].in_project(&repo).payload_bytes());
    let ended = window.agents_until(
        &workspace,
        "the session ended",
        is_state(SessionState::NotRunning),
    );
    assert!(!ended.sessions[0].resumable, "its CLI ended it");

    let refused_in = |window: &mut Window| {
        let asked = window.ask(Request::OpenTerminal {
            workspace: workspace.clone(),
            program: Program::Resume {
                session: session.clone(),
            },
            size: SIZE,
        });
        let Err(Refusal::Failed { reason }) = asked else {
            panic!("the resume was not refused with a reason: {asked:?}");
        };
        assert!(!reason.is_empty());
        // The reply came after all the Keeper does for the request: no terminal is listed
        // and the user's shell never ran.
        assert_eq!(
            window.ask(Request::ListTerminals),
            Ok(Reply::Terminals(vec![]))
        );
        assert!(!scratch.shell_log().exists(), "a launcher ran");
    };
    refused_in(&mut window);
    window.stop(keeper);

    let keeper = scratch.keeper();
    let mut window = scratch.window();
    assert!(!window.agents()[0].sessions[0].resumable);
    refused_in(&mut window);
    window.stop(keeper);
}

#[test]
fn the_first_session_that_starts_in_a_terminal_opened_for_a_review_takes_that_review_and_not_the_one_that_ended_last()
 {
    let scratch = Scratch::with_launcher_shell();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    let dialogs = recorded::hook_events(DIALOGS);
    let tools = recorded::hook_events(TOOLS);
    let at_mention = recorded::hook_events(AT_MENTION);
    let hook = |line: &HookRecord, terminal: Option<&TerminalId>| {
        scratch.hook(terminal, &line.in_project(&repo).input_bytes());
    };

    // Two sessions ran at once, so each has a review; the first ended, then the second. The
    // next session of that CLI would take the review of the one that ended last.
    hook(&dialogs[0], None);
    let first = window.agents_until(&workspace, "the first", is_state(SessionState::Idle));
    let wanted = first.sessions[0].review.clone().expect("a review");
    hook(&tools[1], None);
    window.agents_until(&workspace, "the second", |agents| {
        agents
            .sessions
            .iter()
            .any(|s| s.title == "Claude Code 2" && s.state == SessionState::Idle)
    });
    hook(&dialogs[13], None);
    window.agents_until(
        &workspace,
        "the first ended",
        is_state(SessionState::NotRunning),
    );
    hook(&tools[35], None);
    let ended = window.agents_until(&workspace, "both ended", |agents| {
        agents.sessions.len() == 2
            && agents
                .sessions
                .iter()
                .all(|s| s.state == SessionState::NotRunning)
    });
    let kept_by_second = ended
        .sessions
        .iter()
        .find(|s| s.title == "Claude Code 2")
        .and_then(|s| s.review.clone())
        .expect("a review");
    assert_ne!(wanted, kept_by_second);

    // The terminal is opened for the first one's review, and a session starts in it.
    let terminal = window.open_program(
        &workspace,
        Program::AgentFor {
            agent: yaar_adapter_claude::NAME.to_owned(),
            review: wanted.clone(),
        },
    );
    scratch.launched_with();
    hook(&at_mention[0], Some(&terminal));
    let third = window.agents_until(&workspace, "the third", |agents| {
        agents
            .sessions
            .iter()
            .any(|s| s.title == "Claude Code 3" && s.state == SessionState::Idle)
    });
    let started = third
        .sessions
        .iter()
        .find(|s| s.title == "Claude Code 3")
        .expect("the third");
    assert_eq!(started.review, Some(wanted));
    assert_eq!(started.terminal, Some(terminal));
    let second = third
        .sessions
        .iter()
        .find(|s| s.title == "Claude Code 2")
        .expect("the second");
    assert_eq!(second.review, Some(kept_by_second), "it keeps its own");
    window.stop(keeper);
}

#[test]
fn a_terminal_opened_for_a_review_the_keeper_does_not_hold_is_refused_and_binds_nothing() {
    let scratch = Scratch::with_launcher_shell();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    let dialogs = recorded::hook_events(DIALOGS);
    let at_mention = recorded::hook_events(AT_MENTION);

    // A session ran and ended: the next session of that CLI takes its review.
    scratch.hook(None, &dialogs[0].in_project(&repo).input_bytes());
    let first = window.agents_until(&workspace, "the first", is_state(SessionState::Idle));
    let kept = first.sessions[0].review.clone().expect("a review");
    scratch.hook(None, &dialogs[13].in_project(&repo).input_bytes());
    window.agents_until(
        &workspace,
        "the first ended",
        is_state(SessionState::NotRunning),
    );

    let unheld = ReviewId::new("no-such-review");
    assert_ne!(unheld, kept);
    let asked = window.ask(Request::OpenTerminal {
        workspace: workspace.clone(),
        program: Program::AgentFor {
            agent: yaar_adapter_claude::NAME.to_owned(),
            review: unheld.clone(),
        },
        size: SIZE,
    });
    let Err(Refusal::Failed { reason }) = asked else {
        panic!("the terminal was not refused: {asked:?}");
    };
    assert!(reason.contains("no-such-review"), "{reason}");
    // The reply came after the Keeper had done all it does for the request: no terminal
    // is listed, and the launcher's shell was never run.
    assert_eq!(
        window.ask(Request::ListTerminals),
        Ok(Reply::Terminals(vec![]))
    );
    assert!(
        !scratch.shell_log().exists(),
        "a launcher ran for a review nobody holds"
    );

    // A session that starts in a plain shell of the workspace takes the review the
    // takeover rules give it, and the unheld one is bound to nothing.
    let terminal = window.open_program(&workspace, Program::Shell);
    scratch.hook(
        Some(&terminal),
        &at_mention[0].in_project(&repo).input_bytes(),
    );
    let second = window.agents_until(&workspace, "the second", |agents| {
        agents
            .sessions
            .iter()
            .any(|s| s.title == "Claude Code 2" && s.state == SessionState::Idle)
    });
    let started = second
        .sessions
        .iter()
        .find(|s| s.title == "Claude Code 2")
        .expect("the second");
    assert_eq!(started.review, Some(kept));
    assert_eq!(started.terminal, Some(terminal));
    window.stop(keeper);
}

#[test]
fn a_session_whose_review_a_new_session_took_over_is_no_longer_unreviewed() {
    let mut setup = Setup::new();
    let terminal = setup.window.open_terminal(&setup.workspace);
    let records = recorded::hook_events(TOOLS);
    let dialogs = recorded::hook_events(DIALOGS);
    for line in [1, 2, 32] {
        setup.hook_in(&terminal, &records, line);
    }
    let finished = setup.window.agents_until(
        &setup.workspace,
        "the turn is unreviewed",
        is_state(SessionState::Unreviewed),
    );
    let first = finished.sessions[0].id.clone();
    // The turn's state is recorded by now, so that nothing of the turn is still to come.
    setup.window.until("the end of the turn", |fact| {
        matches!(fact, Fact::TurnEnded { .. }).then_some(())
    });
    setup.hook_in(&terminal, &records, 35);
    setup.window.agents_until(
        &setup.workspace,
        "the session ended",
        is_state(SessionState::NotRunning),
    );

    // A new session in the same terminal takes its review, as after a clear.
    setup.hook_in(&terminal, &dialogs, 14);
    setup
        .window
        .agents_until(&setup.workspace, "the next session", |agents| {
            agents
                .sessions
                .iter()
                .any(|s| s.title == "Claude Code 2" && s.state == SessionState::Idle)
        });

    // Its CLI resumes the first, which has no review of its turn to wait for.
    let elsewhere = setup.window.open_terminal(&setup.workspace);
    setup.hook_in(&elsewhere, &records, 1);
    let resumed = setup
        .window
        .agents_until(&setup.workspace, "the first runs", |agents| {
            agents
                .sessions
                .iter()
                .any(|s| s.id == first && s.state != SessionState::NotRunning)
        });
    let state = resumed
        .sessions
        .iter()
        .find(|s| s.id == first)
        .map(|s| s.state);
    assert_eq!(state, Some(SessionState::Idle));
    assert_eq!(resumed.shown, Some(SessionState::Idle));
    let _ = setup.finish();
}

#[test]
fn an_edit_or_a_write_of_the_agent_after_a_round_marks_the_round_edited_once_and_tells_the_reviews()
{
    let mut setup = Setup::new();
    let terminal = setup.window.open_terminal(&setup.workspace);
    let records = recorded::hook_events(TOOLS);
    setup.hook_in(&terminal, &records, 1);
    let started = setup.window.agents_until(
        &setup.workspace,
        "the session",
        is_state(SessionState::Idle),
    );
    let review = started.sessions[0].review.clone().expect("a review");
    setup.window.draft(&review, 1, 1, "rename this");
    let copied = setup.window.ask(Request::CopyReview {
        review: review.clone(),
    });
    assert!(
        matches!(copied, Ok(Reply::Copied { round: 1, .. })),
        "{copied:?}"
    );
    assert!(!setup.review(&review).rounds[0].edited);

    // A read is no edit. The step is told after the reviews of its item, so that when it is
    // here nothing of the read is still to come.
    let mark = setup.window.facts.len();
    for line in [2, 3, 4] {
        setup.hook_in(&terminal, &records, line);
    }
    setup.step_told("a read", |step| matches!(step, Step::Read { .. }));
    assert_eq!(
        setup.window.reviews_changed_since(mark, &setup.workspace),
        0
    );
    assert!(!setup.review(&review).rounds[0].edited);

    // An edit is.
    let mark = setup.window.facts.len();
    for line in [9, 10] {
        setup.hook_in(&terminal, &records, line);
    }
    setup.step_told("an edit", |step| matches!(step, Step::Edit { .. }));
    assert_eq!(
        setup.window.reviews_changed_since(mark, &setup.workspace),
        1
    );
    assert!(setup.review(&review).rounds[0].edited);

    // The round is edited once: a write after it changes no round and tells nothing.
    let mark = setup.window.facts.len();
    for line in [12, 13] {
        setup.hook_in(&terminal, &records, line);
    }
    setup.step_told("a write", |step| matches!(step, Step::Write { .. }));
    assert_eq!(
        setup.window.reviews_changed_since(mark, &setup.workspace),
        0
    );
    let _ = setup.finish();
}

#[test]
fn the_credits_kept_with_a_turns_state_name_the_session_for_the_file_its_edit_tool_wrote_and_no_one_for_another()
 {
    let mut setup = Setup::new();
    let terminal = setup.window.open_terminal(&setup.workspace);
    let records = recorded::hook_events(TOOLS);
    // Two files that differ from the commit checked out: one the agent's edit tool names,
    // one nobody's tool does.
    let edited = path_of(&records, 10);
    let by_hand = PathBuf::from("by-hand.txt");
    for file in [&edited, &by_hand] {
        let place = setup.repo.join(file);
        std::fs::create_dir_all(place.parent().expect("a folder")).expect("make the folder");
        std::fs::write(place, "changed\n").expect("write the file");
    }
    let committed = head(&setup.repo);

    for line in [1, 2, 9, 10, 32] {
        setup.hook_in(&terminal, &records, line);
    }
    let (session, state) = {
        let ended = setup
            .window
            .until("the end of the turn", |fact| match fact {
                Fact::TurnEnded { session, state, .. } => Some((session.clone(), state.clone())),
                _ => None,
            });
        (ended.0, ended.1.expect("a state is recorded"))
    };
    let review = setup.sessions()[0].review.clone().expect("a review");
    let (store, scratch) = setup.finish();

    // Read once the Keeper has stopped, from what it kept.
    let reviews = Reviews::new(
        store,
        Git::find().expect("git"),
        scratch.0.join("data").join("states"),
    );
    let shown = reviews
        .credited_change(
            &yaar_store::ReviewId::new(review.as_str()),
            &committed,
            &state,
        )
        .expect("the change of the turn");
    let by: Vec<(PathBuf, Vec<String>)> = shown
        .into_iter()
        .map(|change| (change.path, change.by))
        .collect();
    let mut expected = vec![
        (edited, vec![session.as_str().to_owned()]),
        (by_hand, Vec::new()),
    ];
    expected.sort();
    assert_eq!(by, expected);
}

#[test]
fn a_session_that_takes_over_the_review_of_no_agent_tells_the_reviews_changed() {
    let mut setup = Setup::new();
    // A Window that read the reviews holds the review of no agent.
    let held = setup.window.reviews(&setup.workspace).reviews;
    let [review] = &held[..] else {
        panic!("not exactly one review: {held:?}");
    };
    assert_eq!(review.agent, None);
    let review = review.id.clone();
    // Nothing told before this point answers the wait below.
    setup.window.looked_at = setup.window.facts.len();

    let terminal = setup.window.open_terminal(&setup.workspace);
    let records = recorded::hook_events(DIALOGS);
    setup.hook_in(&terminal, &records, 0);
    let workspace = setup.workspace.clone();
    setup.window.until("the reviews told as changed", |fact| {
        matches!(fact, Fact::ReviewsChanged { workspace: told } if *told == workspace).then_some(())
    });

    // A Window that reads them again, as the fact asks, finds the session its owner.
    let session = setup.sessions()[0].id.clone();
    assert_eq!(setup.review(&review).agent, Some(session));
    let _ = setup.finish();
}

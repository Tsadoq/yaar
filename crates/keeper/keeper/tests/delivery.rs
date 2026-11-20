//! Component tests: what the Keeper answers the pieces of Claude Code, the hook program
//! and the reply tool's server, through the surface they use, with a real repository, a real
//! database and the built programs beneath: the review that rides with the user's message,
//! the allows, and the agent's replies.
//!
//! The hook program and the tool server are the built ones, run the way Claude Code runs
//! them, the hook with recorded inputs moved into the test's own worktree; the Keeper is the
//! library on a thread of the test. A review is attached with `Request::AttachReview` and
//! the text it should carry is read with `Request::GetAttachment`, as a Window does, so that
//! what the hook is answered with is compared with what the view control shows. The Keeper runs git with the environment of its process, so each test sets that
//! environment as its first statement, and these tests are run by `cargo nextest`, which gives
//! each test a process of its own.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use yaar_keeper::{Exit, Options, run};
use yaar_link::{
    AgentReport, Anchor, Author, CommentId, DraftOn, Fact, Hello, Lines, Message, Program, Refusal,
    Reply, Request, RequestId, Review, SOCKET_VARIABLE, SentBy, Side, TERMINAL_VARIABLE,
    TerminalId, TerminalSize, ThreadId, TurnId, WorkspaceId, WorkspaceReviews, handshake,
    read_message, write_message,
};
use yaar_store::Store;
use yaar_system_tests::program;
use yaar_system_tests::recorded::{self, HookRecord, PiRecord};

/// How long a test waits for the Keeper before it fails: longer than the plan's ten
/// seconds, as every test file of this crate has it, since a Keeper starts git and shells
/// of its own and a throttled machine runs them slowly.
const DEADLINE: Duration = Duration::from_secs(20);

const SIZE: TerminalSize = TerminalSize {
    columns: 80,
    rows: 24,
};

/// A session with a review, and what the recordings of it hold: `claude/review-file-read.jsonl`
/// is the start (line 0), the user's message (2) and the read of a long review's file (3)
/// of one session.
const REVIEW_FILE: &str = "claude/review-file-read.jsonl";
/// `claude/turn-tools.jsonl`: the start (1) and the user's message (2) of a session, and the
/// message Claude Code started itself (33).
const TURN_TOOLS: &str = "claude/turn-tools.jsonl";
/// `claude/permission-dialogs.jsonl`: a session's start (0), its message (2), the permission
/// request (4), its end by `/clear` (13) and the start of the session that follows (14).
const DIALOGS: &str = "claude/permission-dialogs.jsonl";
/// `claude/reply-tool-hooks.jsonl`: a session's start (1), a read (3) and the `PreToolUse` of
/// a call of the reply tool (11).
const REPLY_HOOKS: &str = "claude/reply-tool-hooks.jsonl";
/// `claude/reply-tool-mcp.jsonl`: what Claude Code sent a tool server; line 7 is a call of
/// the reply tool.
const REPLY_TRACE: &str = "claude/reply-tool-mcp.jsonl";

/// `pi/turn-tools.jsonl`: a session's start (line 1) and its first message (3), the end of
/// its run (28), its end by `/new` (127) and the start of the session that follows in the
/// same terminal and process (129).
const PI_TURN_TOOLS: &str = "pi/turn-tools.jsonl";
/// `pi/queued-message.jsonl`: a session's start (line 1), a message of the user while it is
/// idle (19) and a message pi's own extension queued while a run was open (31).
const PI_QUEUED: &str = "pi/queued-message.jsonl";
/// What the user types while pi works, typed by hand: records 0 to 3 open a run with a shell
/// command that is running, record 4 is the user's message typed then, record 12 the end of
/// the run.
const PI_TYPED_WHILE_WORKING: &str = "pi-typed-while-working.jsonl";

/// A folder of the test's own with a short path, removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    /// Makes the folder and sets the environment of this process for git and for the
    /// programs the Keeper starts: no configuration of the user or of the system, a fixed
    /// author, an empty home. It has to be the first statement of a test.
    fn new() -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        // `/tmp` is a link on macOS, and the Keeper names a worktree by its resolved path.
        let root = Path::new("/tmp")
            .canonicalize()
            .expect("the temporary folder")
            .join(format!("yaar-keeper-delivery-{}-{n}", std::process::id()));
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

    fn socket(&self) -> PathBuf {
        self.0.join("k.sock")
    }

    fn data(&self) -> PathBuf {
        self.0.join("data")
    }

    fn database(&self) -> PathBuf {
        self.data().join(yaar_store::FILE_NAME)
    }

    /// The folder of the Adapter's own reviews of the session `cli` names, where a review
    /// too long to ride inline is written.
    fn reviews_of(&self, cli: &str) -> PathBuf {
        self.data()
            .join("agents")
            .join(yaar_adapter_claude::NAME)
            .join(yaar_adapter_claude::REVIEWS)
            .join(cli)
    }

    /// The folder of the test's own copies of the hook program and the tool server, made at
    /// the first call. Cargo puts a program it built in place again each time, and where
    /// tests run side by side the program is gone for a moment: a copy is made at the first
    /// try that finds it, and is the test's own.
    fn companions(&self) -> PathBuf {
        let folder = self.0.join("companions");
        for (name, built) in [
            (yaar_adapter_claude::HOOK_PROGRAM, "yaar-claude-hook"),
            (yaar_adapter_claude::TOOL_SERVER, "yaar-claude-tools"),
        ] {
            let copy = folder.join(name);
            if copy.is_file() {
                continue;
            }
            std::fs::create_dir_all(&folder).expect("make the folder");
            let deadline = Instant::now() + DEADLINE;
            loop {
                let found = std::panic::catch_unwind(|| program(built));
                if let Ok(found) = found
                    && std::fs::copy(found, &copy).is_ok()
                {
                    break;
                }
                assert!(Instant::now() < deadline, "{built} is never there");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        folder
    }

    /// Starts the Keeper's library on a thread of this test.
    fn keeper(&self) -> JoinHandle<std::io::Result<Exit>> {
        let options = Options {
            socket: self.socket(),
            linger: Duration::from_secs(60),
            app_version: "test-keeper".to_owned(),
            data_dir: self.data(),
            companions: Some(self.companions()),
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

    /// The Keeper's database, opened beside the Keeper. The Store is written for one process
    /// and one connection of the Keeper's, and these tests keep to what it was written for:
    /// they read what the Keeper kept, and write only the one state the app makes nowhere,
    /// while the Keeper is at rest between two hooks.
    fn store(&self) -> Store {
        Store::open(&self.database()).expect("open the Keeper's database")
    }

    /// Runs the hook program with `payload` on its standard input, in the terminal of the
    /// app `terminal`, as Claude Code does. Its parent is this test, the process every piece
    /// of these tests reports as the CLI.
    fn hook(&self, terminal: &TerminalId, payload: &[u8]) -> Answer {
        let mut child = Command::new(self.companions().join(yaar_adapter_claude::HOOK_PROGRAM))
            .env(SOCKET_VARIABLE, self.socket())
            .env(TERMINAL_VARIABLE, terminal.as_str())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the hook program");
        let began = Instant::now();
        child
            .stdin
            .take()
            .expect("its standard input")
            .write_all(payload)
            .expect("hand it the hook's input");
        let output = child.wait_with_output().expect("wait for the hook program");
        let took = began.elapsed();
        assert!(output.status.success(), "the hook program failed");
        Answer {
            printed: output.stdout,
            took,
        }
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

/// What the hook program printed for one hook, and how long it took from the start of the
/// program to its end.
struct Answer {
    printed: Vec<u8>,
    took: Duration,
}

impl Answer {
    /// Whether the hook printed nothing: Claude Code goes on as it would have.
    fn is_nothing(&self) -> bool {
        self.printed.is_empty()
    }

    /// What it printed, as JSON.
    fn json(&self) -> Value {
        serde_json::from_slice(&self.printed).unwrap_or_else(|e| {
            panic!(
                "the hook printed no JSON ({e}): {:?}",
                String::from_utf8_lossy(&self.printed)
            )
        })
    }

    /// The text that goes to the model with the message, which an answer to a message holds.
    fn context(&self) -> String {
        let answer = self.json();
        assert_eq!(
            answer["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit",
            "{answer}"
        );
        answer["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap_or_else(|| panic!("no additionalContext in {answer}"))
            .to_owned()
    }

    /// Whether it is the allow of a read or of a call.
    fn allows(&self) -> bool {
        self.json()["hookSpecificOutput"]["permissionDecision"] == "allow"
    }
}

/// One end of the Link, as a Window holds it. Everything it is sent is kept, so that a test
/// reads it in order, and what it has looked at is not looked at again.
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
        let id = RequestId(self.next_id);
        self.next_id += 1;
        write_message(&mut self.stream, &Message::Request { id, request }).expect("send");
        loop {
            if let Some(at) = self.replies.iter().position(|(to, _)| *to == id) {
                return self.replies.remove(at).1;
            }
            self.read("a reply");
        }
    }

    /// The next `ReviewsChanged` of `workspace` that this window has not looked at.
    fn reviews_changed(&mut self, workspace: &WorkspaceId) {
        loop {
            while self.looked_at < self.facts.len() {
                self.looked_at += 1;
                if matches!(
                    &self.facts[self.looked_at - 1],
                    Fact::ReviewsChanged { workspace: changed } if changed == workspace
                ) {
                    return;
                }
            }
            self.read("a change of the reviews");
        }
    }

    /// Reads until the end of the turn `turn` is told, and gives the state it ended on.
    fn until_turn_ended(&mut self, turn: &TurnId) -> Option<String> {
        loop {
            while self.looked_at < self.facts.len() {
                self.looked_at += 1;
                if let Fact::TurnEnded {
                    turn: ended, state, ..
                } = &self.facts[self.looked_at - 1]
                    && ended == turn
                {
                    return state.clone();
                }
            }
            self.read("the end of a turn");
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

    /// Opens a terminal with a shell in it, and waits until the shell has started: the CLI
    /// whose reports the tests make is typed in a shell of the app, so the Keeper holds the
    /// terminal its reports name.
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
        write_message(
            &mut self.stream,
            &Message::Input {
                terminal: terminal.clone(),
                bytes: b"printf 'ready-%s\\n' now\r".to_vec(),
            },
        )
        .expect("send the bytes");
        self.output_until(&terminal, "ready-now");
        terminal
    }

    fn reviews(&mut self, workspace: &WorkspaceId) -> WorkspaceReviews {
        match self.ask(Request::GetReviews {
            workspace: workspace.clone(),
        }) {
            Ok(Reply::Reviews(reviews)) => reviews,
            other => panic!("no reviews: {other:?}"),
        }
    }

    fn stop(mut self, keeper: JoinHandle<std::io::Result<Exit>>) {
        write_message(&mut self.stream, &Message::Shutdown).expect("ask for the stop");
        assert_eq!(keeper.join().expect("join").expect("run"), Exit::Shutdown);
    }
}

/// A server of the reply tool, started as Claude Code starts it: from the folder its session
/// works in, with the session's id in its environment, its parent being whatever started it.
/// Ended and collected when the test ends, whatever way it does.
struct Tools {
    child: Child,
    lines: Receiver<String>,
    next_id: u32,
}

impl Tools {
    /// The server of the session `cli`, started by this test, which is then its parent.
    fn start(setup: &Setup, cli: &str) -> Tools {
        let mut command = Command::new(
            setup
                .scratch
                .companions()
                .join(yaar_adapter_claude::TOOL_SERVER),
        );
        Tools::run(setup, &mut command, cli)
    }

    /// The same, started by a shell that stays its parent: another process than this test.
    fn start_under_a_shell(setup: &Setup, cli: &str) -> Tools {
        let tools = setup
            .scratch
            .companions()
            .join(yaar_adapter_claude::TOOL_SERVER);
        let mut command = Command::new("/bin/sh");
        // A second command after the server keeps the shell from replacing itself with it.
        command
            .arg("-c")
            .arg(format!("'{}'; exit $?", tools.display()));
        Tools::run(setup, &mut command, cli)
    }

    fn run(setup: &Setup, command: &mut Command, cli: &str) -> Tools {
        let mut child = command
            .env(SOCKET_VARIABLE, setup.scratch.socket())
            .env(TERMINAL_VARIABLE, setup.terminal.as_str())
            .env("CLAUDE_CODE_SESSION_ID", cli)
            .current_dir(&setup.repo)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the tool server");
        let output = child.stdout.take().expect("its standard output");
        let (to_test, lines) = mpsc::channel();
        // Ends when the server's output closes, which is when it ends.
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines().map_while(Result::ok) {
                if to_test.send(line).is_err() {
                    return;
                }
            }
        });
        Tools {
            child,
            lines,
            next_id: 1,
        }
    }

    /// A call of the reply tool as Claude Code sent one (line 7 of its recorded trace), with
    /// the thread, the body and whether it is addressed changed. Gives the call's result.
    fn reply(&mut self, thread: &ThreadId, body: &str, addressed: Option<bool>) -> Value {
        let trace = recorded::mcp_messages(REPLY_TRACE);
        let mut call = trace[7].message.clone();
        assert_eq!(call["method"], "tools/call");
        call["id"] = json!(self.next_id);
        self.next_id += 1;
        call["params"]["arguments"]["thread"] = json!(thread.as_str());
        call["params"]["arguments"]["body"] = json!(body);
        match addressed {
            Some(addressed) => call["params"]["arguments"]["addressed"] = json!(addressed),
            None => {
                call["params"]["arguments"]
                    .as_object_mut()
                    .expect("the arguments")
                    .remove("addressed");
            }
        }
        let stdin = self.child.stdin.as_mut().expect("its standard input");
        writeln!(stdin, "{call}").expect("send the call");
        stdin.flush().expect("send the call");
        let line = self
            .lines
            .recv_timeout(DEADLINE)
            .expect("the tool server answers the call");
        let answer: Value = serde_json::from_str(&line).expect("an answer in JSON");
        assert_eq!(answer["id"], call["id"], "{answer}");
        answer["result"].clone()
    }
}

impl Drop for Tools {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// What the tool server's result says: whether it is an error, and its text.
fn told(result: &Value) -> (bool, &str) {
    (
        result["isError"] == true,
        result["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("no text in {result}")),
    )
}

/// A Keeper with one repository open and a Window attached.
struct Setup {
    scratch: Scratch,
    keeper: JoinHandle<std::io::Result<Exit>>,
    window: Window,
    repo: PathBuf,
    workspace: WorkspaceId,
    terminal: TerminalId,
    /// How many ids have been drawn for threads and comments.
    ids: u128,
}

impl Setup {
    /// Has to be the first statement of a test.
    fn new() -> Setup {
        let scratch = Scratch::new();
        let repo = scratch.repository("repo");
        let keeper = scratch.keeper();
        let mut window = scratch.window();
        let workspace = window.open(&repo);
        let terminal = window.open_terminal(&workspace);
        Setup {
            scratch,
            keeper,
            window,
            repo,
            workspace,
            terminal,
            ids: 0,
        }
    }

    /// Runs the hook of `record`, moved into the repository, and gives what it printed. It
    /// is answered once the Keeper has taken the report, so what the report did is kept by
    /// the time this returns.
    fn hook(&self, record: &HookRecord) -> Answer {
        self.scratch
            .hook(&self.terminal, &record.in_project(&self.repo).input_bytes())
    }

    /// Reports one event of pi, as its reporting program does: the payload as the extension
    /// writes it, from the terminal of the app, of the process of this test. A connection of
    /// its own, as the program makes one for each report, so that the facts this Window
    /// waits for are not read by it.
    fn pi(&self, record: &PiRecord) -> Reply {
        self.pi_from(&self.terminal, std::process::id(), record)
    }

    /// [`Setup::pi`] from another terminal and process.
    fn pi_from(&self, terminal: &TerminalId, pid: u32, record: &PiRecord) -> Reply {
        self.pi_payload(terminal, pid, record.in_project(&self.repo).payload_bytes())
    }

    fn pi_payload(&self, terminal: &TerminalId, pid: u32, payload: Vec<u8>) -> Reply {
        self.scratch
            .window()
            .ask(Request::Report {
                report: AgentReport {
                    agent: yaar_adapter_pi::NAME.to_owned(),
                    terminal: Some(terminal.clone()),
                    pid,
                    payload,
                },
            })
            .unwrap_or_else(|refused| panic!("the report was refused: {refused:?}"))
    }

    /// The review of the session that Claude Code names `cli`.
    fn review_of(&mut self, cli: &str) -> Review {
        self.review_of_agent(yaar_adapter_claude::NAME, cli)
    }

    /// The review of the session that the CLI `agent` names `cli`.
    fn review_of_agent(&mut self, agent: &str, cli: &str) -> Review {
        let session = self
            .scratch
            .store()
            .session_of_cli(agent, cli)
            .expect("read the session")
            .unwrap_or_else(|| panic!("the Keeper holds no session {cli}"));
        let reviews = self.window.reviews(&self.workspace);
        reviews
            .reviews
            .into_iter()
            .find(|review| {
                review.agent.as_ref().map(yaar_link::SessionId::as_str) == Some(session.id.as_str())
            })
            .unwrap_or_else(|| panic!("the session {cli} has no review"))
    }

    fn id(&mut self) -> String {
        self.ids += 1;
        format!("{:032x}", self.ids)
    }

    /// A draft on the first line of `a.txt`, in a thread of its own: the thread and the
    /// comment. The save is told to every Window, and this waits for that, so that what it
    /// tells is not taken for what a later step tells.
    fn draft(&mut self, review: &Review, text: &str) -> (ThreadId, CommentId) {
        let (thread, comment) = (ThreadId::new(self.id()), CommentId::new(self.id()));
        let saved = self.window.ask(Request::SaveDraft {
            review: review.id.clone(),
            thread: thread.clone(),
            comment: comment.clone(),
            on: DraftOn::NewThread {
                anchor: Anchor {
                    tree: None,
                    path: PathBuf::from("a.txt"),
                    side: Side::New,
                    lines: Lines { first: 1, last: 1 },
                    text: "one".to_owned(),
                },
            },
            text: text.to_owned(),
        });
        assert_eq!(saved, Ok(Reply::Done));
        self.window.reviews_changed(&self.workspace);
        (thread, comment)
    }

    /// Attaches `review`, as the user does with the control: the whole review, to the next
    /// message of its session. The attach is told to every Window, and this waits for that,
    /// so that what it tells is not taken for what a later step tells.
    fn attach(&mut self, review: &Review) {
        let attached = self.window.ask(Request::AttachReview {
            review: review.id.clone(),
            thread: None,
        });
        assert!(
            matches!(attached, Ok(Reply::AttachedReview { .. })),
            "{attached:?}"
        );
        self.window.reviews_changed(&self.workspace);
    }

    /// What the next message of the review's session would carry, as the control that views
    /// the attachment shows it: the unsent comments of what is attached after the instruction
    /// of Claude Code's delivery.
    fn attachment(&mut self, review: &Review) -> String {
        let shown = self.window.ask(Request::GetAttachment {
            review: review.id.clone(),
        });
        let Ok(Reply::Attachment { text }) = shown else {
            panic!("no attachment: {shown:?}");
        };
        text
    }

    /// Starts a session by its recorded start, and gives its review, attached and holding a
    /// draft thread: the thread and the text it should be sent as.
    fn session_with_a_draft(
        &mut self,
        start: &HookRecord,
        text: &str,
    ) -> (Review, ThreadId, CommentId) {
        self.hook(start);
        let review = self.review_of(start.session());
        let (thread, comment) = self.draft(&review, text);
        self.attach(&review);
        (review, thread, comment)
    }

    /// [`Setup::session_with_a_draft`] for a session of pi, started by its recorded start.
    fn pi_session_with_a_draft(
        &mut self,
        start: &PiRecord,
        text: &str,
    ) -> (Review, ThreadId, CommentId) {
        assert_eq!(self.pi(start), Reply::Done);
        let review = self.review_of_agent(yaar_adapter_pi::NAME, start.session());
        let (thread, comment) = self.draft(&review, text);
        self.attach(&review);
        (review, thread, comment)
    }

    /// The turn of `review`'s session that runs now, as `ListAgents` tells it; none between
    /// turns.
    fn open_turn_of(&mut self, review: &Review) -> Option<TurnId> {
        let Ok(Reply::Agents(agents)) = self.window.ask(Request::ListAgents) else {
            panic!("no list of agents");
        };
        agents
            .into_iter()
            .flat_map(|listed| listed.sessions)
            .find(|session| session.review.as_ref() == Some(&review.id))
            .unwrap_or_else(|| panic!("no session has the review {}", review.id))
            .turn
    }

    /// Waits for the Keeper to tell that the reviews changed `times` times: the send and
    /// then the state of its round each tell one.
    fn changes(&mut self, times: usize) {
        for _ in 0..times {
            self.window.reviews_changed(&self.workspace);
        }
    }

    /// Waits until every round of the review of the session `cli` holds its state, which is
    /// a repeated look at a fact that only becomes true: the recording thread keeps it off the
    /// agents' way, and no change of the reviews is a barrier for it.
    fn wait_for_states(&mut self, cli: &str) {
        self.wait_for_states_of(yaar_adapter_claude::NAME, cli);
    }

    /// [`Setup::wait_for_states`] for a session of the CLI `agent`.
    fn wait_for_states_of(&mut self, agent: &str, cli: &str) {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let review = self.review_of_agent(agent, cli);
            if !review.rounds.is_empty()
                && review
                    .rounds
                    .iter()
                    .all(|round| round.state.as_ref().is_some_and(|state| !state.is_empty()))
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the rounds of {cli} hold no state: {:?}",
                review.rounds
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Waits until the message of the session `cli` is sent and the state of its round is
    /// kept: the two changes the send tells, and then the fact itself, which holds whatever
    /// else was told before them.
    fn settled(&mut self, cli: &str) {
        self.changes(2);
        self.wait_for_states(cli);
    }

    fn stop(self) {
        self.window.stop(self.keeper);
    }
}

/// The comment `comment` of `review`, wherever it is.
fn comment_of<'a>(review: &'a Review, comment: &CommentId) -> &'a yaar_link::Comment {
    review
        .threads
        .iter()
        .flat_map(|thread| &thread.comments)
        .find(|held| held.id == *comment)
        .unwrap_or_else(|| panic!("the review holds no comment {comment}"))
}

/// The comments of the agent, in every thread of `review`.
fn agent_comments(review: &Review) -> Vec<&yaar_link::Comment> {
    review
        .threads
        .iter()
        .flat_map(|thread| &thread.comments)
        .filter(|comment| comment.author == Author::Agent)
        .collect()
}

#[test]
fn a_message_of_the_user_is_answered_with_the_attached_review_after_the_instruction_and_its_comments_are_sent_in_round_one()
 {
    let mut setup = Setup::new();
    let session = recorded::hook_events(REVIEW_FILE);
    let (review, _, comment) = setup.session_with_a_draft(&session[0], "Why is this `one`?");
    // What the view control shows, read before the message.
    let shown = setup.attachment(&review);
    assert!(
        shown.starts_with(yaar_adapter_claude::INSTRUCTION),
        "{shown}"
    );
    assert!(shown.contains("Why is this `one`?"), "{shown}");

    // The user's message: the recorded prompt.
    let answer = setup.hook(&session[2]);
    let context = answer.context();
    assert_eq!(context, shown);
    assert!(context.starts_with(yaar_adapter_claude::INSTRUCTION));
    assert_eq!(
        answer.json()["systemMessage"],
        "yaar: review round 1 attached (1 threads)"
    );
    // How long the answer took, for the report on a throttled run.
    eprintln!(
        "the answer to the UserPromptSubmit took {:?} from the start of the hook program",
        answer.took
    );

    // The comments are sent: told once for the send and once for the state its round is
    // given after, off the agent's way.
    setup.changes(1);
    let after_the_send = setup.review_of(session[0].session());
    let [round] = &after_the_send.rounds[..] else {
        panic!("not one round: {:?}", after_the_send.rounds);
    };
    assert_eq!((round.number, round.sent_by), (1, SentBy::Delivered));
    assert_eq!(comment_of(&after_the_send, &comment).round, Some(1));
    assert_eq!(after_the_send.attached, None);
    // Whether the state is kept already when the reading above is made depends on how fast the
    // recording thread is, so the reading above shows the send only. The state is looked at
    // until it is there, not counted from the changes that were told.
    setup.wait_for_states(session[0].session());
    setup.stop();
}

#[test]
fn the_instruction_travels_whatever_the_user_wrote() {
    let mut setup = Setup::new();
    let session = recorded::hook_events(REVIEW_FILE);
    let (review, ..) = setup.session_with_a_draft(&session[0], "Rename it.");
    let shown = setup.attachment(&review);

    // The one field changed: the prompt of the recorded message, which is `ok`.
    let mut message = session[2].clone();
    message.input["prompt"] = json!("ok");
    let context = setup.hook(&message).context();
    assert!(
        context.starts_with(yaar_adapter_claude::INSTRUCTION),
        "{context}"
    );
    assert_eq!(context, shown);
    setup.stop();
}

#[test]
fn a_message_that_claude_code_started_itself_is_answered_nothing_and_the_attachment_stays_for_the_users_next()
 {
    let mut setup = Setup::new();
    let session = recorded::hook_events(TURN_TOOLS);
    let (review, _, comment) = setup.session_with_a_draft(&session[1], "Name this.");
    let shown = setup.attachment(&review);

    // The `<task-notification>` prompt of the recording.
    assert!(
        session[33].input["prompt"]
            .as_str()
            .is_some_and(|prompt| prompt.starts_with("<task-notification>"))
    );
    assert!(setup.hook(&session[33]).is_nothing());
    // The report was taken when the hook returned: the review is as it was.
    let kept = setup.review_of(session[1].session());
    assert_eq!(kept.attached, Some(yaar_link::Attachment::Review));
    assert!(kept.rounds.is_empty());
    assert_eq!(comment_of(&kept, &comment).round, None);

    // The user's own message then carries it, which also shows the nothing was not a failure
    // of the delivery.
    assert_eq!(setup.hook(&session[2]).context(), shown);
    setup.stop();
}

#[test]
fn a_permission_request_is_answered_nothing_whatever_is_attached() {
    let mut setup = Setup::new();
    let session = recorded::hook_events(DIALOGS);
    let (review, _, comment) = setup.session_with_a_draft(&session[0], "Why?");
    let shown = setup.attachment(&review);

    assert_eq!(session[4].event, "PermissionRequest");
    assert!(setup.hook(&session[4]).is_nothing());
    let kept = setup.review_of(session[0].session());
    assert_eq!(kept.attached, Some(yaar_link::Attachment::Review));
    assert!(kept.rounds.is_empty());
    assert_eq!(comment_of(&kept, &comment).round, None);
    // It stays for the message the dialog's answer does not run.
    assert_eq!(setup.hook(&session[2]).context(), shown);
    setup.stop();
}

#[test]
fn a_message_of_a_session_whose_review_has_nothing_attached_is_answered_nothing_and_leaves_its_draft()
 {
    let mut setup = Setup::new();
    let session = recorded::hook_events(REVIEW_FILE);
    setup.hook(&session[0]);
    let review = setup.review_of(session[0].session());
    let (_, comment) = setup.draft(&review, "Not attached.");

    assert!(setup.hook(&session[2]).is_nothing());
    let kept = setup.review_of(session[0].session());
    assert!(kept.rounds.is_empty());
    assert_eq!(comment_of(&kept, &comment).round, None);
    setup.stop();
}

#[test]
fn a_message_carries_only_the_review_of_its_own_session() {
    let mut setup = Setup::new();
    let first = recorded::hook_events(REVIEW_FILE);
    let second = recorded::hook_events(TURN_TOOLS);
    // Two sessions that run at once in one worktree have a review each.
    let (review_a, thread_a, _) = setup.session_with_a_draft(&first[0], "For A.");
    setup.hook(&second[1]);
    let review_b = setup.review_of(second[1].session());
    assert_ne!(review_a.id, review_b.id);
    let shown_a = setup.attachment(&review_a);

    // A message of B, which has nothing attached: nothing is answered, and what is attached
    // to A is as it was. B's report was taken when its hook returned.
    assert!(setup.hook(&second[2]).is_nothing());
    let a = setup.review_of(first[0].session());
    assert_eq!(a.attached, Some(yaar_link::Attachment::Review));
    assert!(a.rounds.is_empty());

    // A's message carries A's review, and only it.
    let context = setup.hook(&first[2]).context();
    assert_eq!(context, shown_a);
    assert!(context.contains(thread_a.as_str()));
    setup.settled(first[0].session());
    assert!(setup.review_of(second[1].session()).rounds.is_empty());
    setup.stop();
}

#[test]
fn a_review_that_cannot_be_handed_out_sends_nothing_and_the_comments_stay_attached() {
    let mut setup = Setup::new();
    let session = recorded::hook_events(REVIEW_FILE);
    // The one field changed, in the start and in the message: a session id that is no folder
    // name, which Claude Code's delivery refuses to write a file for.
    let named = |record: &HookRecord| {
        let mut record = record.clone();
        record.input["session_id"] = json!("not a folder name/..");
        record
    };
    let (review, _, comment) = setup.session_with_a_draft(&named(&session[0]), "Cannot go.");
    assert!(review.rounds.is_empty());

    assert!(setup.hook(&named(&session[2])).is_nothing());
    let kept = setup.review_of("not a folder name/..");
    assert_eq!(kept.attached, Some(yaar_link::Attachment::Review));
    assert!(kept.rounds.is_empty());
    assert_eq!(comment_of(&kept, &comment).round, None);
    setup.stop();
}

#[test]
fn a_review_longer_than_the_inline_limit_is_a_file_the_session_may_read_and_no_other_file_is_allowed()
 {
    let mut setup = Setup::new();
    let session = recorded::hook_events(REVIEW_FILE);
    let cli = session[0].session().to_owned();
    // 10,001 characters: the text that carries it is longer than the 10,000 units a hook may
    // answer.
    let long = "x".repeat(10_001);
    let (review, ..) = setup.session_with_a_draft(&session[0], &long);
    let shown = setup.attachment(&review);

    let context = setup.hook(&session[2]).context();
    assert!(context.starts_with(yaar_adapter_claude::INSTRUCTION));
    assert_ne!(context, shown, "the review rode inline");
    // The path the agent is told to read, and the file written whole at it.
    let folder = setup.scratch.reviews_of(&cli);
    let file = context
        .lines()
        .find(|line| line.starts_with(&*folder.to_string_lossy()))
        .unwrap_or_else(|| {
            panic!(
                "the pointer names no file under {}: {context}",
                folder.display()
            )
        });
    assert_eq!(
        std::fs::read_to_string(file).expect("the file is written"),
        shown
    );

    // The recorded `PreToolUse` of `Read`, with the one field changed that names a file.
    assert_eq!(session[3].input["tool_name"], "Read");
    let read = |setup: &Setup, file: &Path| {
        let mut record = session[3].clone();
        record.input["tool_input"]["file_path"] = json!(file.to_string_lossy());
        setup.hook(&record)
    };
    assert!(read(&setup, Path::new(file)).allows());
    // A file of the worktree, a file of another session's folder of reviews, a path that
    // leads out of the session's own folder through its parent, and the folder itself: the
    // CLI asks about each.
    let another = setup.scratch.reviews_of("another-session");
    std::fs::create_dir_all(&another).expect("make the folder");
    std::fs::write(another.join("r-1.md"), "not yours").expect("write the file");
    let out_through_the_parent = folder.join("..").join("another-session").join("r-1.md");
    for not_allowed in [
        setup.repo.join("a.txt"),
        another.join("r-1.md"),
        out_through_the_parent,
        folder.clone(),
    ] {
        assert!(
            read(&setup, &not_allowed).is_nothing(),
            "allowed: {}",
            not_allowed.display()
        );
    }
    setup.stop();
}

#[test]
fn the_call_of_the_reply_tool_is_allowed_and_a_call_of_another_tool_is_not() {
    let setup = Setup::new();
    let session = recorded::hook_events(REPLY_HOOKS);
    setup.hook(&session[1]);

    // The `PreToolUse` of the call of the app's tool, as the recording names it once its
    // working names are the app's.
    assert!(
        session[11].input["tool_name"]
            .as_str()
            .is_some_and(|name| name == yaar_adapter_claude::LISTED_REPLY_TOOL)
    );
    assert!(setup.hook(&session[11]).allows());
    // The `PreToolUse` of a read of a file of the project asks the CLI's own question.
    assert!(setup.hook(&session[3]).is_nothing());
    // One field changed: another server's tool.
    let mut other = session[11].clone();
    other.input["tool_name"] = json!("mcp__plugin_other_server__reply_to_thread");
    assert!(setup.hook(&other).is_nothing());
    setup.stop();
}

#[test]
fn a_reply_of_the_tool_server_is_kept_in_its_thread_and_answered_with_its_result() {
    let mut setup = Setup::new();
    let session = recorded::hook_events(REVIEW_FILE);
    let cli = session[0].session().to_owned();
    let (review, first, _) = setup.session_with_a_draft(&session[0], "First.");
    let (second, _) = setup.draft(&review, "Second.");
    setup.hook(&session[2]);
    setup.settled(&cli);

    let mut tools = Tools::start(&setup, &cli);
    // A body with quotes, a line break and characters that are more bytes than they are
    // characters: it is kept as it is and counted in characters.
    let body = "It's \"done\": see `a.txt`.\nSecond line, with ünïcödé and 日本語.";
    let result = tools.reply(&first, body, Some(true));
    assert_eq!(
        told(&result),
        (
            false,
            format!(
                "recorded reply to {first} (addressed, {} characters)",
                body.chars().count()
            )
            .as_str()
        )
    );
    assert_ne!(body.chars().count(), body.len());
    // The reply is told to every Window, and is kept in its thread, in the round the review
    // is in, as the comment of the agent.
    setup.changes(1);
    let kept = setup.review_of(&cli);
    let thread = kept
        .threads
        .iter()
        .find(|t| t.id == first)
        .expect("the thread");
    let [.., reply] = &thread.comments[..] else {
        panic!("no comments");
    };
    assert_eq!(
        (reply.author, reply.text.as_str(), reply.round),
        (Author::Agent, body, Some(1))
    );
    assert!(thread.addressed);

    // Left out, the mark is not given: the thread is open.
    let result = tools.reply(&second, "Not sure.", None);
    assert_eq!(
        told(&result),
        (
            false,
            format!("recorded reply to {second} (open, 9 characters)").as_str()
        )
    );
    setup.changes(1);
    let kept = setup.review_of(&cli);
    assert!(
        !kept
            .threads
            .iter()
            .find(|t| t.id == second)
            .expect("the thread")
            .addressed
    );
    assert_eq!(agent_comments(&kept).len(), 2);
    setup.stop();
}

#[test]
fn a_reply_naming_the_thread_of_another_sessions_review_is_answered_with_an_error_and_keeps_nothing()
 {
    let mut setup = Setup::new();
    let first = recorded::hook_events(REVIEW_FILE);
    let second = recorded::hook_events(TURN_TOOLS);
    let cli_a = first[0].session().to_owned();
    let cli_b = second[1].session().to_owned();
    let (_, thread_a, _) = setup.session_with_a_draft(&first[0], "For A.");
    let (_, thread_b, _) = setup.session_with_a_draft(&second[1], "For B.");
    setup.hook(&first[2]);
    setup.hook(&second[2]);
    setup.changes(4);
    setup.wait_for_states(&cli_a);
    setup.wait_for_states(&cli_b);

    // A's server names B's thread.
    let mut tools = Tools::start(&setup, &cli_a);
    let result = tools.reply(&thread_b, "Not mine.", Some(true));
    assert_eq!(
        told(&result),
        (true, format!("there is no thread {thread_b}").as_str())
    );
    // It was taken when the result came: nothing of the agent is in either review.
    assert_eq!(agent_comments(&setup.review_of(&cli_a)).len(), 0);
    assert_eq!(agent_comments(&setup.review_of(&cli_b)).len(), 0);

    // Its own thread is kept, so the refusal was not a server that cannot reach the Keeper.
    let result = tools.reply(&thread_a, "Mine.", Some(false));
    assert!(!told(&result).0, "{result}");
    assert_eq!(agent_comments(&setup.review_of(&cli_a)).len(), 1);
    assert_eq!(agent_comments(&setup.review_of(&cli_b)).len(), 0);
    setup.stop();
}

#[test]
fn after_a_clear_a_reply_of_the_server_that_names_the_cleared_session_is_kept_in_the_review_the_new_session_took_over()
 {
    let mut setup = Setup::new();
    let session = recorded::hook_events(DIALOGS);
    let cleared = session[0].session().to_owned();
    let new = session[14].session().to_owned();
    assert_ne!(cleared, new);
    let (review, thread, _) = setup.session_with_a_draft(&session[0], "Before the clear.");
    setup.hook(&session[2]);
    setup.settled(&cleared);

    // The recorded end of that session by `/clear`, and the start of the session that
    // follows in the same terminal and process.
    assert_eq!(session[13].input["reason"], "clear");
    setup.hook(&session[13]);
    setup.hook(&session[14]);
    let taken = setup.review_of(&new);
    assert_eq!(taken.id, review.id, "the new session took the review over");

    // The server of Claude Code was started before the clear and still names the old
    // session. It is Claude Code's, and so is its parent: this test, as the hooks' is.
    let mut tools = Tools::start(&setup, &cleared);
    let result = tools.reply(&thread, "After the clear.", Some(true));
    assert!(!told(&result).0, "{result}");
    setup.changes(1);
    let kept = setup.review_of(&new);
    assert_eq!(kept.id, review.id);
    let [reply] = &agent_comments(&kept)[..] else {
        panic!("not one reply: {:?}", agent_comments(&kept));
    };
    assert_eq!(reply.text, "After the clear.");
    setup.stop();
}

#[test]
fn after_a_clear_a_reply_naming_the_cleared_session_from_another_process_is_refused() {
    let mut setup = Setup::new();
    let session = recorded::hook_events(DIALOGS);
    let cleared = session[0].session().to_owned();
    let (_, thread, _) = setup.session_with_a_draft(&session[0], "Before the clear.");
    setup.hook(&session[2]);
    setup.settled(&cleared);
    setup.hook(&session[13]);
    setup.hook(&session[14]);

    // Started by a shell, so that its parent is not the process the new session runs in.
    let mut tools = Tools::start_under_a_shell(&setup, &cleared);
    let result = tools.reply(&thread, "From elsewhere.", Some(true));
    let (is_error, why) = told(&result);
    assert!(
        is_error && why.starts_with("there is no review of the session"),
        "{result}"
    );
    assert_eq!(
        agent_comments(&setup.review_of(session[14].session())).len(),
        0
    );
    setup.stop();
}

#[test]
fn a_reply_naming_a_session_that_has_ended_with_its_review_is_kept_in_the_session_of_its_process_that_runs()
 {
    let mut setup = Setup::new();
    let first = recorded::hook_events(REVIEW_FILE);
    let second = recorded::hook_events(TURN_TOOLS);
    let cli_a = first[0].session().to_owned();
    let cli_b = second[1].session().to_owned();
    // Two sessions of one process, each with a review and a thread.
    let (review_a, _, _) = setup.session_with_a_draft(&first[0], "For A.");
    let (review_b, thread_b, _) = setup.session_with_a_draft(&second[1], "For B.");
    setup.hook(&second[2]);
    setup.settled(&cli_b);

    // The recorded end of A, whose reason is not `clear`: A runs no more and keeps its
    // review, which only a session that starts later may take.
    assert_eq!(first[7].event, "SessionEnd");
    assert_ne!(first[7].input["reason"], "clear");
    setup.hook(&first[7]);
    assert_eq!(setup.review_of(&cli_a).id, review_a.id);

    // A server that still names A, from the process B runs in: the report is of B, so the
    // thread of B's review is found, which A's review does not hold.
    let mut tools = Tools::start(&setup, &cli_a);
    let result = tools.reply(&thread_b, "For the one that runs.", Some(true));
    assert!(!told(&result).0, "{result}");
    setup.changes(1);
    let kept_b = setup.review_of(&cli_b);
    assert_eq!(kept_b.id, review_b.id);
    assert_eq!(agent_comments(&kept_b).len(), 1);
    assert_eq!(agent_comments(&setup.review_of(&cli_a)).len(), 0);
    setup.stop();
}

#[test]
fn a_reply_naming_a_session_that_runs_with_no_review_is_kept_in_the_session_of_its_process_that_has_one()
 {
    let mut setup = Setup::new();
    let first = recorded::hook_events(REVIEW_FILE);
    let second = recorded::hook_events(TURN_TOOLS);
    let cli_a = first[0].session().to_owned();
    let cli_b = second[1].session().to_owned();
    let (review_a, thread_a, _) = setup.session_with_a_draft(&first[0], "For A.");
    let (review_b, ..) = setup.session_with_a_draft(&second[1], "For B.");
    // A's comment is sent, which a reply to its thread needs.
    setup.hook(&first[2]);
    setup.settled(&cli_a);

    // B runs, and its review is of no agent: the Store allows the state, the app makes it
    // nowhere today, and the Keeper's rule says whose a report that names B then is.
    setup
        .scratch
        .store()
        .set_review_agent(&yaar_store::ReviewId::new(review_b.id.as_str()), None)
        .expect("give B's review to no agent");

    // A server that names B, from the process A runs in as well: B has no review, so the
    // report is of A.
    let mut tools = Tools::start(&setup, &cli_b);
    let result = tools.reply(&thread_a, "For the one that has a review.", Some(true));
    assert!(!told(&result).0, "{result}");
    setup.changes(1);
    let kept_a = setup.review_of(&cli_a);
    assert_eq!(kept_a.id, review_a.id);
    assert_eq!(agent_comments(&kept_a).len(), 1);
    setup.stop();
}

/// The one round `review` holds, sent by delivery.
fn delivered_round(review: &Review) -> &yaar_link::Round {
    let [round] = &review.rounds[..] else {
        panic!("not one round: {:?}", review.rounds);
    };
    assert_eq!((round.number, round.sent_by), (1, SentBy::Delivered));
    round
}

#[test]
fn a_message_of_a_pi_session_is_answered_with_the_attached_review_after_the_instruction_and_its_comments_are_sent_in_the_turn_it_opened()
 {
    let mut setup = Setup::new();
    let run = recorded::pi_events(PI_TURN_TOOLS);
    let (review, _, comment) = setup.pi_session_with_a_draft(&run[1], "Why is this `one`?");
    // What the view control shows, read before the message.
    let shown = setup.attachment(&review);
    assert!(shown.starts_with(yaar_adapter_pi::INSTRUCTION), "{shown}");
    assert!(shown.contains("Why is this `one`?"), "{shown}");
    assert_eq!(setup.open_turn_of(&review), None);

    // The user's message: the recorded `input`, which opens the run.
    assert_eq!(run[3].event, "input");
    let Reply::Answer { bytes } = setup.pi(&run[3]) else {
        panic!("the message was not answered with the review");
    };
    let answer: Value = serde_json::from_slice(&bytes).expect("the answer is JSON");
    assert_eq!(answer, json!({ "review": shown }));

    // The comments are sent, in round one, whose turn is the one the message opened.
    let sent = setup.review_of_agent(yaar_adapter_pi::NAME, run[1].session());
    let round = delivered_round(&sent);
    assert_eq!(comment_of(&sent, &comment).round, Some(1));
    assert_eq!(sent.attached, None);
    let opened = setup
        .open_turn_of(&review)
        .expect("the message opened a turn");
    assert_eq!(round.turn, Some(opened));
    setup.stop();
}

#[test]
fn the_round_of_a_message_of_claude_code_is_sent_in_the_turn_the_message_opened_and_a_copy_has_none()
 {
    let mut setup = Setup::new();
    let session = recorded::hook_events(REVIEW_FILE);
    let (review, ..) = setup.session_with_a_draft(&session[0], "Why is this `one`?");
    assert_eq!(setup.open_turn_of(&review), None);
    setup.hook(&session[2]);
    setup.settled(session[0].session());
    let sent = setup.review_of(session[0].session());
    let opened = setup
        .open_turn_of(&review)
        .expect("the message opened a turn");
    assert_eq!(delivered_round(&sent).turn, Some(opened));

    // A second draft, copied as text: no turn runs the copy, whatever turn is open.
    setup.draft(&sent, "And this?");
    let Ok(Reply::Copied { round, .. }) = setup.window.ask(Request::CopyReview {
        review: review.id.clone(),
    }) else {
        panic!("the review was not copied");
    };
    assert_eq!(round, 2);
    let copied = setup.review_of(session[0].session());
    let [first, second] = &copied.rounds[..] else {
        panic!("not two rounds: {:?}", copied.rounds);
    };
    assert_eq!(
        (first.turn.clone(), second.sent_by),
        (sent.rounds[0].turn.clone(), SentBy::Copied)
    );
    assert!(first.turn.is_some());
    assert_eq!(second.turn, None);
    setup.stop();
}

/// The hand-made records of a message typed while pi works, with the review attached after
/// the run opened, and what the Keeper answers the message with: the run opened by record 1,
/// the answer to record 4, and the Window after the whole run, which is told the run's end.
/// `behavior` is the one field of record 4 changed, `streamingBehavior`, where it is given.
fn a_message_typed_while_pi_works(behavior: Option<&str>) {
    let mut setup = Setup::new();
    let records = recorded::hand_made_pi_events(PI_TYPED_WHILE_WORKING);
    assert_eq!(records[4].event, "input");
    let mut typed = records[4].clone();
    if let Some(behavior) = behavior {
        typed.payload["streamingBehavior"] = json!(behavior);
    }
    assert_eq!(
        typed.payload["streamingBehavior"],
        behavior.unwrap_or("steer")
    );

    // The run is open, a shell command of it running: the session start, the message that
    // opened it (nothing was attached), the run's start and the call of the tool.
    assert_eq!(setup.pi(&records[0]), Reply::Done);
    let review = setup.review_of_agent(yaar_adapter_pi::NAME, records[0].session());
    for record in &records[1..=3] {
        assert_eq!(setup.pi(record), Reply::Done, "{}", record.event);
    }
    let run = setup
        .open_turn_of(&review)
        .expect("the first message opened a turn");
    // The review is attached while pi works.
    let (_, comment) = setup.draft(&review, "Look at this while you work.");
    setup.attach(&review);
    let shown = setup.attachment(&review);

    // The message typed then goes with the review, at the report that pi took it.
    let Reply::Answer { bytes } = setup.pi(&typed) else {
        panic!("the message that pi queued was not answered with the review");
    };
    let answer: Value = serde_json::from_slice(&bytes).expect("the answer is JSON");
    assert_eq!(answer, json!({ "review": shown }));
    assert!(shown.starts_with(yaar_adapter_pi::INSTRUCTION), "{shown}");
    // Marked sent now, when pi reported the message: the rest of the run is not reported
    // yet. The round's turn is the run that was open, and no turn started for the message.
    let sent = setup.review_of_agent(yaar_adapter_pi::NAME, records[0].session());
    let round = delivered_round(&sent);
    assert_eq!(comment_of(&sent, &comment).round, Some(1));
    assert_eq!(sent.attached, None);
    assert_eq!(round.turn, Some(run.clone()));
    assert_eq!(setup.open_turn_of(&review), Some(run.clone()));

    // The rest of the run, which asks and gives nothing, up to the end of the run.
    for record in &records[5..=12] {
        assert_eq!(setup.pi(record), Reply::Done, "{}", record.event);
    }
    assert_eq!(records[12].event, "agent_settled");
    assert!(
        setup.window.until_turn_ended(&run).is_some(),
        "the run that was open ended with a state"
    );
    let after = setup.review_of_agent(yaar_adapter_pi::NAME, records[0].session());
    // The state of the round is recorded off the agent's way, so it is not compared.
    assert_eq!(
        delivered_round(&after).turn,
        Some(run),
        "no other round was sent"
    );
    assert_eq!(setup.open_turn_of(&review), None);
    setup.stop();
}

#[test]
fn a_message_pi_takes_while_it_works_carries_the_review_and_the_comments_are_sent_at_its_report() {
    a_message_typed_while_pi_works(None);
}

#[test]
fn a_message_pi_queues_for_after_its_run_carries_the_review_the_same_way() {
    a_message_typed_while_pi_works(Some("followUp"));
}

#[test]
fn a_message_pi_queued_by_its_own_extension_is_answered_nothing_and_the_attachment_stays_for_the_users_next()
 {
    let mut setup = Setup::new();
    let queued = recorded::pi_events(PI_QUEUED);
    let (review, _, comment) = setup.pi_session_with_a_draft(&queued[1], "Name this.");
    let shown = setup.attachment(&review);

    assert_eq!(queued[31].payload["source"], "extension");
    assert_eq!(setup.pi(&queued[31]), Reply::Done);
    // The report was taken when it was answered: the review is as it was.
    let kept = setup.review_of_agent(yaar_adapter_pi::NAME, queued[1].session());
    assert_eq!(kept.attached, Some(yaar_link::Attachment::Review));
    assert!(kept.rounds.is_empty());
    assert_eq!(comment_of(&kept, &comment).round, None);

    // The user's own message then carries it, which also shows the nothing was not a
    // failure of the delivery.
    assert_eq!(queued[19].payload["source"], "interactive");
    let Reply::Answer { bytes } = setup.pi(&queued[19]) else {
        panic!("the user's message was not answered with the review");
    };
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).expect("JSON"),
        json!({ "review": shown })
    );
    setup.stop();
}

/// The payload of a call of pi's reply tool, as the extension writes it, in the session `of`
/// names.
fn reply_payload(of: &PiRecord, setup: &Setup, thread: &ThreadId, body: &str) -> Vec<u8> {
    let of = of.in_project(&setup.repo);
    serde_json::to_vec(&json!({
        "event": yaar_adapter_pi::REPLY_EVENT,
        "sessionId": of.session(),
        "sessionFile": of.session_file(),
        "cwd": of.payload["cwd"],
        "toolCallId": "call_1",
        "arguments": { "thread": thread.as_str(), "body": body, "addressed": true },
    }))
    .expect("a payload")
}

/// What the app answered a call of the reply tool, as the extension reads it.
fn tool_answer(reply: Reply) -> Value {
    let Reply::Answer { bytes } = reply else {
        panic!("the call was answered with no bytes: {reply:?}");
    };
    serde_json::from_slice(&bytes).expect("the answer is JSON")
}

#[test]
fn a_reply_of_pi_is_kept_in_its_thread_and_answered_with_its_result_and_one_naming_another_sessions_thread_is_answered_with_an_error()
 {
    let mut setup = Setup::new();
    let first = recorded::pi_events(PI_TURN_TOOLS);
    let second = recorded::pi_events(PI_QUEUED);
    // Two sessions in two terminals, each with a thread of its own, A's comment sent.
    let (review_a, thread_a, _) = setup.pi_session_with_a_draft(&first[1], "For A.");
    let other_terminal = setup.window.open_terminal(&setup.workspace);
    // Another process that is alive for as long as the test runs: this one's parent.
    // SAFETY: `getppid` takes no argument and cannot fail.
    let other_pid = unsafe { libc::getppid() }.unsigned_abs();
    assert_eq!(
        setup.pi_from(&other_terminal, other_pid, &second[1]),
        Reply::Done
    );
    let review_b = setup.review_of_agent(yaar_adapter_pi::NAME, second[1].session());
    let (thread_b, _) = setup.draft(&review_b, "For B.");
    setup.attach(&review_b);
    assert!(matches!(setup.pi(&first[3]), Reply::Answer { .. }));
    assert!(matches!(
        setup.pi_from(&other_terminal, other_pid, &second[19]),
        Reply::Answer { .. }
    ));

    // A reply with a hostile body, kept as it was written.
    let body = "Done: `rename`, \"quoted\", $(touch PWNED)\nsecond line\\";
    let answered = tool_answer(setup.pi_payload(
        &setup.terminal.clone(),
        std::process::id(),
        reply_payload(&first[1], &setup, &thread_a, body),
    ));
    assert_eq!(
        answered,
        json!({
            "text": format!(
                "recorded reply to {thread_a} (addressed, {} characters)",
                body.chars().count()
            )
        })
    );
    let kept = setup.review_of_agent(yaar_adapter_pi::NAME, first[1].session());
    assert_eq!(kept.id, review_a.id);
    let [reply] = &agent_comments(&kept)[..] else {
        panic!("not one reply: {:?}", agent_comments(&kept));
    };
    assert_eq!(reply.text, body);
    let thread = kept
        .threads
        .iter()
        .find(|thread| thread.id == thread_a)
        .expect("the thread");
    assert!(thread.addressed);
    assert!(!thread.resolved, "a reply never resolves");

    // A reply of B's session naming A's thread: an error for the tool, nothing kept.
    let refused = tool_answer(setup.pi_payload(
        &other_terminal,
        other_pid,
        reply_payload(&second[1], &setup, &thread_a, "Not mine."),
    ));
    assert_eq!(
        refused,
        json!({ "error": format!("there is no thread {thread_a}") })
    );
    assert_eq!(
        agent_comments(&setup.review_of_agent(yaar_adapter_pi::NAME, second[1].session())).len(),
        0
    );
    assert_eq!(
        agent_comments(&setup.review_of_agent(yaar_adapter_pi::NAME, first[1].session())).len(),
        1
    );
    // Its own thread is kept, so the refusal was not a Keeper that cannot be reached.
    let kept_b = tool_answer(setup.pi_payload(
        &other_terminal,
        other_pid,
        reply_payload(&second[1], &setup, &thread_b, "Mine."),
    ));
    assert!(kept_b["text"].is_string(), "{kept_b}");
    assert!(!setup.repo.join("PWNED").exists());
    setup.stop();
}

#[test]
fn after_pis_new_a_reply_of_the_new_session_is_kept_in_the_review_it_took_over() {
    let mut setup = Setup::new();
    let run = recorded::pi_events(PI_TURN_TOOLS);
    let (review, thread, _) = setup.pi_session_with_a_draft(&run[1], "Before the new session.");
    assert!(matches!(setup.pi(&run[3]), Reply::Answer { .. }));
    assert_eq!(setup.pi(&run[28]), Reply::Done);

    // The recorded end of that session by `/new`, and the start of the session that follows
    // in the same terminal and process.
    assert_eq!(run[127].event, "session_shutdown");
    assert_eq!(run[127].payload["reason"], "new");
    assert_eq!(setup.pi(&run[127]), Reply::Done);
    assert_eq!(run[129].event, "session_start");
    assert_ne!(run[129].session(), run[1].session());
    assert_eq!(setup.pi(&run[129]), Reply::Done);
    let taken = setup.review_of_agent(yaar_adapter_pi::NAME, run[129].session());
    assert_eq!(taken.id, review.id, "the new session took the review over");

    // The agent calls the tool in the new session: its id is the new one.
    let answered = tool_answer(setup.pi_payload(
        &setup.terminal.clone(),
        std::process::id(),
        reply_payload(&run[129], &setup, &thread, "After the new session."),
    ));
    assert!(answered["text"].is_string(), "{answered}");
    let kept = setup.review_of_agent(yaar_adapter_pi::NAME, run[129].session());
    assert_eq!(kept.id, review.id);
    let [reply] = &agent_comments(&kept)[..] else {
        panic!("not one reply: {:?}", agent_comments(&kept));
    };
    assert_eq!(reply.text, "After the new session.");
    setup.stop();
}

#[test]
fn the_turn_a_round_was_sent_in_is_told_again_after_the_keeper_is_started_again() {
    let mut setup = Setup::new();
    let run = recorded::pi_events(PI_TURN_TOOLS);
    let (review, ..) = setup.pi_session_with_a_draft(&run[1], "Why is this `one`?");
    assert!(matches!(setup.pi(&run[3]), Reply::Answer { .. }));
    let turn = delivered_round(&setup.review_of_agent(yaar_adapter_pi::NAME, run[1].session()))
        .turn
        .clone()
        .expect("the round has a turn");
    // The state of the round is recorded off the agent's way, and a Keeper stopped before it
    // is would show none: wait for it, so that what is read after is the whole round.
    setup.wait_for_states_of(yaar_adapter_pi::NAME, run[1].session());
    let Setup {
        scratch,
        keeper,
        window,
        workspace,
        ..
    } = setup;
    window.stop(keeper);

    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let reviews = window.reviews(&workspace);
    let kept = reviews
        .reviews
        .iter()
        .find(|held| held.id == review.id)
        .expect("the review is kept");
    assert_eq!(delivered_round(kept).turn, Some(turn));
    window.stop(keeper);
}

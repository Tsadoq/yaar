//! Each test sets the process environment first, so each needs a process of its own (cargo
//! nextest).

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

const DEADLINE: Duration = Duration::from_secs(20);

const SIZE: TerminalSize = TerminalSize {
    columns: 80,
    rows: 24,
};

/// Start (line 0), the user's message (2) and the read of a long review's file (3) of one session.
const REVIEW_FILE: &str = "claude/review-file-read.jsonl";
/// Start (1) and the user's message (2) of a session, and the message Claude Code started itself
/// (33).
const TURN_TOOLS: &str = "claude/turn-tools.jsonl";
/// A session's start (0), its message (2), the permission request (4), its end by `/clear` (13)
/// and the start of the session that follows (14).
const DIALOGS: &str = "claude/permission-dialogs.jsonl";
/// A session's start (1), a read (3) and the `PreToolUse` of a call of the reply tool (11).
const REPLY_HOOKS: &str = "claude/reply-tool-hooks.jsonl";
/// What Claude Code sent a tool server; line 7 is a call of the reply tool.
const REPLY_TRACE: &str = "claude/reply-tool-mcp.jsonl";

/// A session's start (1) and first message (3), the end of its run (28), its end by `/new` (127)
/// and the start of the session that follows in the same terminal and process (129).
const PI_TURN_TOOLS: &str = "pi/turn-tools.jsonl";
/// A session's start (1), a user message while it is idle (19) and a message pi's own extension
/// queued while a run was open (31).
const PI_QUEUED: &str = "pi/queued-message.jsonl";
/// A hand-made recording: records 0 to 3 open a run with a running shell command, record 4
/// is the user's message typed then, record 12 the end of the run.
const PI_TYPED_WHILE_WORKING: &str = "pi-typed-while-working.jsonl";

struct Scratch(PathBuf);

impl Scratch {
    /// Sets the process environment; must be a test's first statement.
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

    fn reviews_of(&self, cli: &str) -> PathBuf {
        self.data()
            .join("agents")
            .join(yaar_adapter_claude::NAME)
            .join(yaar_adapter_claude::REVIEWS)
            .join(cli)
    }

    /// The test's own copies of the hook program and the tool server: cargo replaces a program on
    /// each build, so under parallel tests it is briefly missing.
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

    fn keeper(&self) -> JoinHandle<std::io::Result<Exit>> {
        let options = Options {
            socket: self.socket(),
            linger: Duration::from_secs(60),
            app_version: "test-keeper".to_owned(),
            data_dir: self.data(),
            companions: Some(self.companions()),
            home: self.0.join("home"),
            login_shell: None,
        };
        std::thread::spawn(move || run(options))
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

    fn store(&self) -> Store {
        Store::open(&self.database()).expect("open the Keeper's database")
    }

    /// Its parent is this test, the process every piece of these tests reports as the CLI.
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

struct Answer {
    printed: Vec<u8>,
    took: Duration,
}

impl Answer {
    fn is_nothing(&self) -> bool {
        self.printed.is_empty()
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.printed).unwrap_or_else(|e| {
            panic!(
                "the hook printed no JSON ({e}): {:?}",
                String::from_utf8_lossy(&self.printed)
            )
        })
    }

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

    fn allows(&self) -> bool {
        self.json()["hookSpecificOutput"]["permissionDecision"] == "allow"
    }
}

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

    fn output_until(&mut self, terminal: &TerminalId, text: &str) {
        loop {
            let heard = self.output.iter().find(|(from, _)| from == terminal);
            if heard.is_some_and(|(_, bytes)| String::from_utf8_lossy(bytes).contains(text)) {
                return;
            }
            self.read(text);
        }
    }

    /// The CLI whose reports the tests make is typed in a shell of the app, so the Keeper holds the
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

struct Tools {
    child: Child,
    lines: Receiver<String>,
    next_id: u32,
}

impl Tools {
    fn start(setup: &Setup, cli: &str) -> Tools {
        let mut command = Command::new(
            setup
                .scratch
                .companions()
                .join(yaar_adapter_claude::TOOL_SERVER),
        );
        Tools::run(setup, &mut command, cli)
    }

    /// Started by a shell that stays its parent: another process than this test.
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

fn told(result: &Value) -> (bool, &str) {
    (
        result["isError"] == true,
        result["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("no text in {result}")),
    )
}

struct Setup {
    scratch: Scratch,
    keeper: JoinHandle<std::io::Result<Exit>>,
    window: Window,
    repo: PathBuf,
    workspace: WorkspaceId,
    terminal: TerminalId,
    ids: u128,
}

impl Setup {
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

    fn hook(&self, record: &HookRecord) -> Answer {
        self.scratch
            .hook(&self.terminal, &record.in_project(&self.repo).input_bytes())
    }

    /// Reports one pi event on a connection of its own, as pi's reporting program does, so it
    /// reads none of the facts this Window waits for.
    fn pi(&self, record: &PiRecord) -> Reply {
        self.pi_from(&self.terminal, std::process::id(), record)
    }

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

    fn review_of(&mut self, cli: &str) -> Review {
        self.review_of_agent(yaar_adapter_claude::NAME, cli)
    }

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

    fn attachment(&mut self, review: &Review) -> String {
        let shown = self.window.ask(Request::GetAttachment {
            review: review.id.clone(),
        });
        let Ok(Reply::Attachment { text }) = shown else {
            panic!("no attachment: {shown:?}");
        };
        text
    }

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

    fn changes(&mut self, times: usize) {
        for _ in 0..times {
            self.window.reviews_changed(&self.workspace);
        }
    }

    /// Waits until every round of the session's review holds its state. The state is recorded off
    /// the agents' way and no change of the reviews marks it, so this polls until the deadline.
    fn wait_for_states(&mut self, cli: &str) {
        self.wait_for_states_of(yaar_adapter_claude::NAME, cli);
    }

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

    fn settled(&mut self, cli: &str) {
        self.changes(2);
        self.wait_for_states(cli);
    }

    fn stop(self) {
        self.window.stop(self.keeper);
    }
}

fn comment_of<'a>(review: &'a Review, comment: &CommentId) -> &'a yaar_link::Comment {
    review
        .threads
        .iter()
        .flat_map(|thread| &thread.comments)
        .find(|held| held.id == *comment)
        .unwrap_or_else(|| panic!("the review holds no comment {comment}"))
}

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
    let shown = setup.attachment(&review);
    assert!(
        shown.starts_with(yaar_adapter_claude::INSTRUCTION),
        "{shown}"
    );
    assert!(shown.contains("Why is this `one`?"), "{shown}");

    let answer = setup.hook(&session[2]);
    let context = answer.context();
    assert_eq!(context, shown);
    assert!(context.starts_with(yaar_adapter_claude::INSTRUCTION));
    assert_eq!(
        answer.json()["systemMessage"],
        "yaar: review round 1 attached (1 threads)"
    );
    eprintln!(
        "the answer to the UserPromptSubmit took {:?} from the start of the hook program",
        answer.took
    );

    setup.changes(1);
    let after_the_send = setup.review_of(session[0].session());
    let [round] = &after_the_send.rounds[..] else {
        panic!("not one round: {:?}", after_the_send.rounds);
    };
    assert_eq!((round.number, round.sent_by), (1, SentBy::Delivered));
    assert_eq!(comment_of(&after_the_send, &comment).round, Some(1));
    assert_eq!(after_the_send.attached, None);
    // Whether the state is kept when the reading above is made depends on the recording thread, so
    // that reading shows the send only. The state is looked at until it is there.
    setup.wait_for_states(session[0].session());
    setup.stop();
}

#[test]
fn the_instruction_travels_whatever_the_user_wrote() {
    let mut setup = Setup::new();
    let session = recorded::hook_events(REVIEW_FILE);
    let (review, ..) = setup.session_with_a_draft(&session[0], "Rename it.");
    let shown = setup.attachment(&review);

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

    assert!(
        session[33].input["prompt"]
            .as_str()
            .is_some_and(|prompt| prompt.starts_with("<task-notification>"))
    );
    assert!(setup.hook(&session[33]).is_nothing());
    // The report was taken when the hook returned, so the review can be read at once.
    let kept = setup.review_of(session[1].session());
    assert_eq!(kept.attached, Some(yaar_link::Attachment::Review));
    assert!(kept.rounds.is_empty());
    assert_eq!(comment_of(&kept, &comment).round, None);

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
    let (review_a, thread_a, _) = setup.session_with_a_draft(&first[0], "For A.");
    setup.hook(&second[1]);
    let review_b = setup.review_of(second[1].session());
    assert_ne!(review_a.id, review_b.id);
    let shown_a = setup.attachment(&review_a);

    assert!(setup.hook(&second[2]).is_nothing());
    let a = setup.review_of(first[0].session());
    assert_eq!(a.attached, Some(yaar_link::Attachment::Review));
    assert!(a.rounds.is_empty());

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
    // A session id that is no folder name: Claude Code's delivery refuses to write a file for it.
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
    let long = "x".repeat(10_001);
    let (review, ..) = setup.session_with_a_draft(&session[0], &long);
    let shown = setup.attachment(&review);

    let context = setup.hook(&session[2]).context();
    assert!(context.starts_with(yaar_adapter_claude::INSTRUCTION));
    assert_ne!(context, shown, "the review rode inline");
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

    assert_eq!(session[3].input["tool_name"], "Read");
    let read = |setup: &Setup, file: &Path| {
        let mut record = session[3].clone();
        record.input["tool_input"]["file_path"] = json!(file.to_string_lossy());
        setup.hook(&record)
    };
    assert!(read(&setup, Path::new(file)).allows());
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

    assert!(
        session[11].input["tool_name"]
            .as_str()
            .is_some_and(|name| name == yaar_adapter_claude::LISTED_REPLY_TOOL)
    );
    assert!(setup.hook(&session[11]).allows());
    assert!(setup.hook(&session[3]).is_nothing());
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

    let mut tools = Tools::start(&setup, &cli_a);
    let result = tools.reply(&thread_b, "Not mine.", Some(true));
    assert_eq!(
        told(&result),
        (true, format!("there is no thread {thread_b}").as_str())
    );
    // The reply was taken when the tool's result came: no wait is needed.
    assert_eq!(agent_comments(&setup.review_of(&cli_a)).len(), 0);
    assert_eq!(agent_comments(&setup.review_of(&cli_b)).len(), 0);

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

    assert_eq!(session[13].input["reason"], "clear");
    setup.hook(&session[13]);
    setup.hook(&session[14]);
    let taken = setup.review_of(&new);
    assert_eq!(taken.id, review.id, "the new session took the review over");

    // Claude Code's server started before the clear still names the old session.
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
    let (review_a, _, _) = setup.session_with_a_draft(&first[0], "For A.");
    let (review_b, thread_b, _) = setup.session_with_a_draft(&second[1], "For B.");
    setup.hook(&second[2]);
    setup.settled(&cli_b);

    assert_eq!(first[7].event, "SessionEnd");
    assert_ne!(first[7].input["reason"], "clear");
    setup.hook(&first[7]);
    assert_eq!(setup.review_of(&cli_a).id, review_a.id);

    // The server still names A, from the process B runs in: the report is of B, so B's review is
    // found.
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
    setup.hook(&first[2]);
    setup.settled(&cli_a);

    setup
        .scratch
        .store()
        .set_review_agent(&yaar_store::ReviewId::new(review_b.id.as_str()), None)
        .expect("give B's review to no agent");

    let mut tools = Tools::start(&setup, &cli_b);
    let result = tools.reply(&thread_a, "For the one that has a review.", Some(true));
    assert!(!told(&result).0, "{result}");
    setup.changes(1);
    let kept_a = setup.review_of(&cli_a);
    assert_eq!(kept_a.id, review_a.id);
    assert_eq!(agent_comments(&kept_a).len(), 1);
    setup.stop();
}

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
    let shown = setup.attachment(&review);
    assert!(shown.starts_with(yaar_adapter_pi::INSTRUCTION), "{shown}");
    assert!(shown.contains("Why is this `one`?"), "{shown}");
    assert_eq!(setup.open_turn_of(&review), None);

    assert_eq!(run[3].event, "input");
    let Reply::Answer { bytes } = setup.pi(&run[3]) else {
        panic!("the message was not answered with the review");
    };
    let answer: Value = serde_json::from_slice(&bytes).expect("the answer is JSON");
    assert_eq!(answer, json!({ "review": shown }));

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

    assert_eq!(setup.pi(&records[0]), Reply::Done);
    let review = setup.review_of_agent(yaar_adapter_pi::NAME, records[0].session());
    for record in &records[1..=3] {
        assert_eq!(setup.pi(record), Reply::Done, "{}", record.event);
    }
    let run = setup
        .open_turn_of(&review)
        .expect("the first message opened a turn");
    let (_, comment) = setup.draft(&review, "Look at this while you work.");
    setup.attach(&review);
    let shown = setup.attachment(&review);

    let Reply::Answer { bytes } = setup.pi(&typed) else {
        panic!("the message that pi queued was not answered with the review");
    };
    let answer: Value = serde_json::from_slice(&bytes).expect("the answer is JSON");
    assert_eq!(answer, json!({ "review": shown }));
    assert!(shown.starts_with(yaar_adapter_pi::INSTRUCTION), "{shown}");
    let sent = setup.review_of_agent(yaar_adapter_pi::NAME, records[0].session());
    let round = delivered_round(&sent);
    assert_eq!(comment_of(&sent, &comment).round, Some(1));
    assert_eq!(sent.attached, None);
    assert_eq!(round.turn, Some(run.clone()));
    assert_eq!(setup.open_turn_of(&review), Some(run.clone()));

    for record in &records[5..=12] {
        assert_eq!(setup.pi(record), Reply::Done, "{}", record.event);
    }
    assert_eq!(records[12].event, "agent_settled");
    assert!(
        setup.window.until_turn_ended(&run).is_some(),
        "the run that was open ended with a state"
    );
    let after = setup.review_of_agent(yaar_adapter_pi::NAME, records[0].session());
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
    let kept = setup.review_of_agent(yaar_adapter_pi::NAME, queued[1].session());
    assert_eq!(kept.attached, Some(yaar_link::Attachment::Review));
    assert!(kept.rounds.is_empty());
    assert_eq!(comment_of(&kept, &comment).round, None);

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

    assert_eq!(run[127].event, "session_shutdown");
    assert_eq!(run[127].payload["reason"], "new");
    assert_eq!(setup.pi(&run[127]), Reply::Done);
    assert_eq!(run[129].event, "session_start");
    assert_ne!(run[129].session(), run[1].session());
    assert_eq!(setup.pi(&run[129]), Reply::Done);
    let taken = setup.review_of_agent(yaar_adapter_pi::NAME, run[129].session());
    assert_eq!(taken.id, review.id, "the new session took the review over");

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
    // The state of the round is recorded off the agent's way: wait for it, as a Keeper stopped
    // before it is would show none.
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

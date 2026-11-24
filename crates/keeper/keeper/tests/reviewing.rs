//! Component tests of a review through the Window surface. Each test sets the process
//! environment first, so nextest runs it in a process of its own.

use std::io::Write as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_keeper::{Exit, Options, run};
use yaar_link::{
    AgentCli, Anchor, Attachment, CommentId, DraftOn, Fact, Hello, Lines, Message, Program,
    Refusal, Reply, Request, RequestId, ReviewId, SOCKET_VARIABLE, SessionState, Side,
    TERMINAL_VARIABLE, TerminalId, ThreadId, ThreadState, WorkspaceAgents, WorkspaceId,
    WorkspaceReviews, handshake, read_message, write_message,
};
use yaar_recorder::Git;
use yaar_review_engine::{Reviews, ThreadReply};
use yaar_store::Store;
use yaar_system_tests::program;
use yaar_system_tests::recorded::{self, HookRecord};

mod support;

use support::{Environment, SIZE, Scratch, git, is_state};

const DEADLINE: Duration = Duration::from_secs(20);

const DIALOGS: &str = "claude/permission-dialogs.jsonl";

const LINES: [&str; 3] = ["one", "two", "three"];

impl Scratch {
    fn new() -> Scratch {
        Scratch::in_tmp("reviewing", Environment::Full)
    }

    fn socket(&self) -> PathBuf {
        self.0.join("k.sock")
    }

    fn database(&self) -> PathBuf {
        self.0.join("data").join(yaar_store::FILE_NAME)
    }

    /// The folder of the test's own copy of the hook program: cargo puts a rebuilt program in
    /// place again for every test process, so under parallel tests it is briefly missing.
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

    fn keeper(&self) -> JoinHandle<std::io::Result<Exit>> {
        self.keeper_told(Some(self.companions()))
    }

    fn keeper_told(&self, companions: Option<PathBuf>) -> JoinHandle<std::io::Result<Exit>> {
        let options = Options {
            socket: self.socket(),
            linger: Duration::from_secs(60),
            app_version: "test-keeper".to_owned(),
            data_dir: self.0.join("data"),
            companions,
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

    /// `terminal` None is a CLI outside the app. Returns how long it took.
    fn hook(&self, terminal: Option<&TerminalId>, payload: &[u8]) -> Duration {
        let mut command = Command::new(self.companions().join(yaar_adapter_claude::HOOK_PROGRAM));
        command
            .env(SOCKET_VARIABLE, self.socket())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        match terminal {
            Some(terminal) => command.env(TERMINAL_VARIABLE, terminal.as_str()),
            None => command.env_remove(TERMINAL_VARIABLE),
        };
        let began = Instant::now();
        let mut child = command.spawn().expect("start the hook program");
        child
            .stdin
            .take()
            .expect("its standard input")
            .write_all(payload)
            .expect("hand it the hook's input");
        let output = child.wait_with_output().expect("wait for the hook program");
        let took = began.elapsed();
        assert!(output.status.success(), "the hook program failed");
        assert!(output.stdout.is_empty(), "the hook program printed");
        took
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

    fn send(&mut self, request: Request) -> RequestId {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        write_message(&mut self.stream, &Message::Request { id, request }).expect("send");
        id
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
        let id = self.send(request);
        loop {
            if let Some(at) = self.replies.iter().position(|(to, _)| *to == id) {
                return self.replies.remove(at).1;
            }
            self.read("a reply");
        }
    }

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
    fn reviews(&mut self, workspace: &WorkspaceId) -> WorkspaceReviews {
        match self.ask(Request::GetReviews {
            workspace: workspace.clone(),
        }) {
            Ok(Reply::Reviews(reviews)) => reviews,
            other => panic!("no reviews: {other:?}"),
        }
    }

    /// `line` counts from 1.
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

    fn attachment(&mut self, review: &ReviewId) -> Result<Reply, Refusal> {
        self.ask(Request::GetAttachment {
            review: review.clone(),
        })
    }

    /// A reply comes after the facts of the requests before it, so reading up to it gets them all.
    fn flush(&mut self) {
        let listed = self.ask(Request::ListWorkspaces);
        assert!(matches!(listed, Ok(Reply::Workspaces(_))), "{listed:?}");
    }

    fn reviews_changed_since(&self, mark: usize, workspace: &WorkspaceId) -> usize {
        self.facts[mark..]
            .iter()
            .filter(|fact| matches!(fact, Fact::ReviewsChanged { workspace: told } if told == workspace))
            .count()
    }

    /// Reads until an `AgentsChanged` of `workspace` comes after `mark`. It cannot end a count:
    /// a wrongly caused one looks the same, so a counting test ends on `agents_in_state_since`.
    fn agents_told_since(&mut self, mark: usize, workspace: &WorkspaceId) {
        while self.agents_changed_since(mark, workspace) == 0 {
            self.read("the sessions told again");
        }
    }

    /// A hook returns once the agents' side took the report, so the `AgentsChanged` showing `state`
    /// follows every earlier request's.
    fn agents_in_state_since(
        &mut self,
        mark: usize,
        workspace: &WorkspaceId,
        state: SessionState,
    ) -> usize {
        loop {
            let seen = self.facts[mark..].iter().position(|fact| {
                matches!(fact, Fact::AgentsChanged(told) if told.workspace == *workspace
                    && told.sessions.iter().any(|session| session.state == state))
            });
            if let Some(at) = seen {
                return mark + at + 1;
            }
            self.read("the sessions told in the state");
        }
    }

    fn agents_changed_between(&self, mark: usize, end: usize, workspace: &WorkspaceId) -> usize {
        self.facts[mark..end]
            .iter()
            .filter(
                |fact| matches!(fact, Fact::AgentsChanged(told) if told.workspace == *workspace),
            )
            .count()
    }

    fn agents_changed_since(&self, mark: usize, workspace: &WorkspaceId) -> usize {
        self.facts[mark..]
            .iter()
            .filter(
                |fact| matches!(fact, Fact::AgentsChanged(told) if told.workspace == *workspace),
            )
            .count()
    }
}

fn ids(n: u32) -> (ThreadId, CommentId) {
    (
        ThreadId::new(format!("{:032x}", 0x1000 + n)),
        CommentId::new(format!("{:032x}", 0x2000 + n)),
    )
}

struct Setup {
    scratch: Scratch,
    keeper: JoinHandle<std::io::Result<Exit>>,
    window: Window,
    repo: PathBuf,
    workspace: WorkspaceId,
}

impl Setup {
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

    fn hook_in(&self, terminal: &TerminalId, records: &[HookRecord], line: usize) {
        let record = records[line].in_project(&self.repo);
        self.scratch.hook(Some(terminal), &record.input_bytes());
    }

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

    fn review_of_no_agent(&mut self) -> ReviewId {
        let reviews = self.window.reviews(&self.workspace);
        let [review] = &reviews.reviews[..] else {
            panic!("not one review: {:?}", reviews.reviews);
        };
        review.id.clone()
    }

    fn attach(&mut self, review: &ReviewId, thread: Option<&ThreadId>) -> Result<Reply, Refusal> {
        self.window.ask(Request::AttachReview {
            review: review.clone(),
            thread: thread.cloned(),
        })
    }

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

    fn finish(self) -> (Store, Scratch) {
        self.window.stop(self.keeper);
        let store = self.scratch.store();
        (store, self.scratch)
    }
}

fn claude() -> AgentCli {
    AgentCli {
        agent: yaar_adapter_claude::NAME.to_owned(),
        title: yaar_adapter_claude::TITLE.to_owned(),
    }
}

fn pi() -> AgentCli {
    AgentCli {
        agent: yaar_adapter_pi::NAME.to_owned(),
        title: yaar_adapter_pi::TITLE.to_owned(),
    }
}

fn state_of(review: &yaar_link::Review, thread: &ThreadId) -> ThreadState {
    review
        .threads
        .iter()
        .find(|held| held.id == *thread)
        .unwrap_or_else(|| panic!("no thread {thread:?} in the review"))
        .state
}

#[test]
fn attaching_a_review_whose_session_runs_offers_nothing_and_tells_the_reviews_and_the_agents() {
    let mut setup = Setup::new();
    let (_, review, _) = setup.session();
    setup.window.draft(&review, 1, 1, "first draft");
    setup.window.draft(&review, 2, 2, "second draft");
    setup.window.flush();
    let mark = setup.window.facts.len();

    assert_eq!(
        setup.attach(&review, None),
        Ok(Reply::AttachedReview { start: Vec::new() })
    );
    setup.window.flush();
    setup.window.agents_told_since(mark, &setup.workspace);
    assert_eq!(
        setup.window.reviews_changed_since(mark, &setup.workspace),
        1
    );
    assert_eq!(setup.review(&review).attached, Some(Attachment::Review));

    let Ok(Reply::Attachment { text }) = setup.window.attachment(&review) else {
        panic!("no text of the attachment");
    };
    assert!(text.starts_with(yaar_adapter_claude::INSTRUCTION), "{text}");
    assert!(text.contains("first draft") && text.contains("second draft"));
    let _ = setup.finish();
}

#[test]
fn one_thread_attached_alone_carries_that_thread_and_a_later_attach_of_the_review_replaces_it() {
    let mut setup = Setup::new();
    let (_, review, _) = setup.session();
    let (first, _) = setup.window.draft(&review, 1, 1, "first draft");
    setup.window.draft(&review, 2, 2, "second draft");

    assert_eq!(
        setup.attach(&review, Some(&first)),
        Ok(Reply::AttachedReview { start: Vec::new() })
    );
    setup.window.flush();
    assert_eq!(
        setup.review(&review).attached,
        Some(Attachment::Thread(first.clone()))
    );
    let Ok(Reply::Attachment { text }) = setup.window.attachment(&review) else {
        panic!("no text of the attachment");
    };
    assert!(text.starts_with(yaar_adapter_claude::INSTRUCTION), "{text}");
    assert!(text.contains("first draft"), "{text}");
    assert!(!text.contains("second draft"), "{text}");

    assert_eq!(
        setup.attach(&review, None),
        Ok(Reply::AttachedReview { start: Vec::new() })
    );
    assert_eq!(setup.review(&review).attached, Some(Attachment::Review));
    let Ok(Reply::Attachment { text }) = setup.window.attachment(&review) else {
        panic!("no text of the attachment");
    };
    assert!(text.contains("first draft") && text.contains("second draft"));
    let _ = setup.finish();
}

#[test]
fn attaching_the_review_of_a_session_that_ended_offers_its_cli_while_another_session_runs() {
    let mut setup = Setup::new();
    let (first, review, first_terminal) = setup.session();
    setup.window.draft(&review, 1, 1, "first draft");
    let records = recorded::hook_events(DIALOGS);
    let second_terminal = setup.window.open_terminal(&setup.workspace);
    setup.hook_in(&second_terminal, &records, 14);
    setup
        .window
        .agents_until(&setup.workspace, "the second session", |agents| {
            agents.sessions.len() == 2
        });
    setup.hook_in(&first_terminal, &records, 13);
    setup.window.agents_until(
        &setup.workspace,
        "the first session ended",
        is_state(SessionState::NotRunning),
    );
    let sessions = setup.sessions();
    let held = sessions
        .iter()
        .find(|session| session.id == first)
        .expect("the first session is listed");
    assert_eq!(held.state, SessionState::NotRunning);
    assert_eq!(held.review, Some(review.clone()), "it keeps its review");
    assert!(
        sessions
            .iter()
            .any(|session| session.state == SessionState::Idle
                && session.review.is_some()
                && session.review != Some(review.clone())),
        "the other session runs with a review of its own: {sessions:?}"
    );

    assert_eq!(
        setup.attach(&review, None),
        Ok(Reply::AttachedReview {
            start: vec![claude()]
        })
    );
    let _ = setup.finish();
}

#[test]
fn attaching_the_review_of_no_agent_offers_every_supported_cli() {
    let mut setup = Setup::new();
    let review = setup.review_of_no_agent();
    setup.window.draft(&review, 1, 1, "first draft");

    assert_eq!(
        setup.attach(&review, None),
        Ok(Reply::AttachedReview {
            start: vec![claude(), pi()]
        })
    );
    assert_eq!(setup.review(&review).attached, Some(Attachment::Review));
    assert_eq!(
        setup.window.attachment(&review),
        Err(Refusal::NothingToSend {
            review: review.clone()
        })
    );
    let _ = setup.finish();
}

#[test]
fn attaching_the_review_of_a_session_of_a_cli_with_no_adapter_offers_nothing_and_carries_no_text() {
    let mut setup = Setup::new();
    let review = setup.review_of_no_agent();
    setup.window.draft(&review, 1, 1, "first draft");
    let workspace = setup.workspace.clone();
    let (store, scratch) = setup.finish();

    // A CLI without an Adapter, as an older Keeper left it: ended, and the review is its own.
    let session = yaar_store::SessionId::new("5".repeat(32));
    store
        .put_session(&yaar_store::SessionRecord {
            id: session.clone(),
            workspace: yaar_store::WorkspaceId::new(workspace.as_str()),
            agent: "otheragent".to_owned(),
            cli_session: "otheragent-session".to_owned(),
            terminal: None,
            number: 1,
            running: false,
            ended: Some(1),
            ended_by: Some(yaar_store::EndedBy::Cli),
        })
        .expect("keep the session");
    store
        .set_review_agent(&yaar_store::ReviewId::new(review.as_str()), Some(&session))
        .expect("give the review to the session");
    drop(store);

    let keeper = scratch.keeper();
    let mut window = scratch.window();
    assert_eq!(
        window.ask(Request::AttachReview {
            review: review.clone(),
            thread: None,
        }),
        Ok(Reply::AttachedReview { start: Vec::new() })
    );
    assert_eq!(
        window.attachment(&review),
        Err(Refusal::NothingToSend {
            review: review.clone()
        })
    );
    window.stop(keeper);
}

#[test]
fn an_attach_that_names_what_is_not_there_or_carries_nothing_is_refused_and_tells_nobody_and_leaves_what_was_attached()
 {
    let mut setup = Setup::new();
    let (_, review, terminal) = setup.session();
    let (resolved, _) = setup.window.draft(&review, 1, 1, "to resolve");
    assert_eq!(
        setup.window.ask(Request::ResolveThread {
            review: review.clone(),
            thread: resolved.clone(),
        }),
        Ok(Reply::Done)
    );
    setup.window.flush();
    let mark = setup.window.facts.len();

    let nobody = ReviewId::new("0".repeat(32));
    let nowhere = ThreadId::new("0".repeat(32));
    assert_eq!(
        setup.attach(&nobody, None),
        Err(Refusal::UnknownReview {
            review: nobody.clone()
        })
    );
    assert_eq!(
        setup.attach(&review, Some(&nowhere)),
        Err(Refusal::UnknownThread {
            thread: nowhere.clone()
        })
    );
    for thread in [None, Some(&resolved)] {
        assert_eq!(
            setup.attach(&review, thread),
            Err(Refusal::NothingToSend {
                review: review.clone()
            }),
            "{thread:?}"
        );
    }
    assert_eq!(
        setup.window.attachment(&review),
        Err(Refusal::NothingToSend {
            review: review.clone()
        })
    );
    assert_eq!(setup.review(&review).attached, None);
    assert_eq!(
        setup.window.attachment(&nobody),
        Err(Refusal::UnknownReview {
            review: nobody.clone()
        })
    );

    setup.window.draft(&review, 2, 2, "to keep");
    assert_eq!(
        setup.attach(&review, None),
        Ok(Reply::AttachedReview { start: Vec::new() })
    );
    setup.window.flush();
    assert_eq!(
        setup.window.reviews_changed_since(mark, &setup.workspace),
        2,
        "the draft and the attach tell, the refusals do not"
    );

    assert_eq!(
        setup.attach(&review, Some(&resolved)),
        Err(Refusal::NothingToSend {
            review: review.clone()
        })
    );
    assert_eq!(setup.review(&review).attached, Some(Attachment::Review));
    let Ok(Reply::Attachment { text }) = setup.window.attachment(&review) else {
        panic!("the attachment is gone");
    };
    assert!(
        text.contains("to keep") && !text.contains("to resolve"),
        "{text}"
    );

    // Barrier: a permission request goes through the agents' side after everything asked before it,
    // so its `AgentsChanged` is the last the earlier requests could cause.
    let records = recorded::hook_events(DIALOGS);
    assert_eq!(records[4].event, "PermissionRequest");
    setup.hook_in(&terminal, &records, 4);
    let end = setup.window.agents_in_state_since(
        mark,
        &setup.workspace,
        SessionState::WaitsForPermission,
    );
    assert_eq!(
        setup
            .window
            .agents_changed_between(mark, end - 1, &setup.workspace),
        1,
        "only the attach tells the agents"
    );
    let _ = setup.finish();
}

#[test]
fn a_detach_takes_the_attachment_away_keeps_every_draft_and_tells_the_reviews_and_the_agents() {
    let mut setup = Setup::new();
    let (_, review, terminal) = setup.session();
    setup.window.draft(&review, 1, 1, "first draft");
    setup.window.draft(&review, 2, 2, "second draft");
    setup.window.flush();
    let before = setup.window.facts.len();
    assert_eq!(
        setup.attach(&review, None),
        Ok(Reply::AttachedReview { start: Vec::new() })
    );
    setup.window.agents_told_since(before, &setup.workspace);
    setup.window.flush();
    let mark = setup.window.facts.len();

    // Reading tells nobody, so the counts below are the detach's own.
    let read = setup.window.attachment(&review);
    assert!(matches!(read, Ok(Reply::Attachment { .. })), "{read:?}");
    assert_eq!(
        setup.window.ask(Request::DetachReview {
            review: review.clone()
        }),
        Ok(Reply::Done)
    );
    setup.window.flush();
    assert_eq!(
        setup.window.reviews_changed_since(mark, &setup.workspace),
        1
    );
    let records = recorded::hook_events(DIALOGS);
    setup.hook_in(&terminal, &records, 2);
    let end = setup
        .window
        .agents_in_state_since(mark, &setup.workspace, SessionState::Works);
    assert_eq!(
        setup
            .window
            .agents_changed_between(mark, end - 1, &setup.workspace),
        1
    );
    let held = setup.review(&review);
    assert_eq!(held.attached, None);
    assert_eq!(held.threads.len(), 2);
    for thread in &held.threads {
        assert_eq!(thread.state, ThreadState::Draft);
        assert_eq!(thread.comments[0].round, None, "still an unsent draft");
    }
    assert_eq!(
        setup.window.attachment(&review),
        Err(Refusal::NothingToSend {
            review: review.clone()
        })
    );

    assert_eq!(
        setup.window.ask(Request::DetachReview {
            review: review.clone()
        }),
        Ok(Reply::Done)
    );
    let nobody = ReviewId::new("0".repeat(32));
    assert_eq!(
        setup.window.ask(Request::DetachReview {
            review: nobody.clone()
        }),
        Err(Refusal::UnknownReview { review: nobody })
    );
    let _ = setup.finish();
}

#[test]
fn a_thread_is_resolved_and_reopened_by_the_reviewer_alone_and_one_attached_alone_stays_attached() {
    let mut setup = Setup::new();
    let (_, review, _) = setup.session();
    let (first, _) = setup.window.draft(&review, 1, 1, "first draft");
    let (second, _) = setup.window.draft(&review, 2, 2, "second draft");
    assert_eq!(
        setup.attach(&review, Some(&first)),
        Ok(Reply::AttachedReview { start: Vec::new() })
    );
    setup.window.flush();
    let mark = setup.window.facts.len();

    assert_eq!(
        setup.window.ask(Request::ResolveThread {
            review: review.clone(),
            thread: first.clone(),
        }),
        Ok(Reply::Done)
    );
    setup.window.flush();
    assert_eq!(
        setup.window.reviews_changed_since(mark, &setup.workspace),
        1
    );
    let held = setup.review(&review);
    assert!(held.threads.iter().any(|t| t.id == first && t.resolved));
    assert_eq!(state_of(&held, &first), ThreadState::Resolved);
    assert_eq!(state_of(&held, &second), ThreadState::Draft);
    assert_eq!(held.attached, Some(Attachment::Thread(first.clone())));
    assert_eq!(
        setup.window.attachment(&review),
        Err(Refusal::NothingToSend {
            review: review.clone()
        })
    );

    let mark = setup.window.facts.len();
    assert_eq!(
        setup.window.ask(Request::ReopenThread {
            review: review.clone(),
            thread: first.clone(),
        }),
        Ok(Reply::Done)
    );
    setup.window.flush();
    assert_eq!(
        setup.window.reviews_changed_since(mark, &setup.workspace),
        1
    );
    let held = setup.review(&review);
    assert!(held.threads.iter().all(|t| !t.resolved));
    assert_eq!(state_of(&held, &first), ThreadState::Draft);
    let Ok(Reply::Attachment { text }) = setup.window.attachment(&review) else {
        panic!("the reopened thread carries nothing");
    };
    assert!(text.contains("first draft"), "{text}");

    let other_repo = setup.scratch.repository("other");
    let other_workspace = setup.window.open(&other_repo);
    let other_review = {
        let reviews = setup.window.reviews(&other_workspace);
        reviews.reviews[0].id.clone()
    };
    let (foreign, _) = setup.window.draft(&other_review, 9, 1, "foreign draft");
    setup.window.flush();
    let mark = setup.window.facts.len();
    let nobody = ReviewId::new("0".repeat(32));
    let nowhere = ThreadId::new("0".repeat(32));
    for (request, refusal) in [
        (
            Request::ResolveThread {
                review: review.clone(),
                thread: nowhere.clone(),
            },
            Refusal::UnknownThread {
                thread: nowhere.clone(),
            },
        ),
        (
            Request::ReopenThread {
                review: review.clone(),
                thread: foreign.clone(),
            },
            Refusal::UnknownThread {
                thread: foreign.clone(),
            },
        ),
        (
            Request::ResolveThread {
                review: review.clone(),
                thread: foreign.clone(),
            },
            Refusal::UnknownThread {
                thread: foreign.clone(),
            },
        ),
        (
            Request::ResolveThread {
                review: nobody.clone(),
                thread: first.clone(),
            },
            Refusal::UnknownReview {
                review: nobody.clone(),
            },
        ),
        (
            Request::ReopenThread {
                review: nobody.clone(),
                thread: first.clone(),
            },
            Refusal::UnknownReview {
                review: nobody.clone(),
            },
        ),
    ] {
        assert_eq!(
            setup.window.ask(request.clone()),
            Err(refusal),
            "{request:?}"
        );
    }
    setup.window.flush();
    assert_eq!(
        setup.window.reviews_changed_since(mark, &setup.workspace)
            + setup.window.reviews_changed_since(mark, &other_workspace),
        0
    );
    let untouched = setup.window.reviews(&other_workspace).reviews;
    assert_eq!(state_of(&untouched[0], &foreign), ThreadState::Draft);
    let _ = setup.finish();
}

struct Answered {
    _scratch: Scratch,
    keeper: JoinHandle<std::io::Result<Exit>>,
    window: Window,
    workspace: WorkspaceId,
    review: ReviewId,
    sent: ThreadId,
    addressed: ThreadId,
    waiting: ThreadId,
    resolved: ThreadId,
    also_addressed: ThreadId,
}

impl Answered {
    /// Has to be the first statement of a test. Five threads are sent in one round and one is
    /// resolved; the agent's replies are kept through the engine while the Keeper is stopped.
    fn new() -> Answered {
        let mut setup = Setup::new();
        let (session, review, _) = setup.session();
        let (sent, _) = setup.window.draft(&review, 1, 1, "sent");
        let (addressed, _) = setup.window.draft(&review, 2, 2, "addressed");
        let (waiting, _) = setup.window.draft(&review, 3, 3, "waiting");
        let (resolved, _) = setup.window.draft(&review, 4, 1, "resolved");
        let (also_addressed, _) = setup.window.draft(&review, 5, 2, "also addressed");
        let copied = setup.window.ask(Request::CopyReview {
            review: review.clone(),
        });
        assert!(
            matches!(copied, Ok(Reply::Copied { round: 1, .. })),
            "{copied:?}"
        );
        assert_eq!(
            setup.window.ask(Request::ResolveThread {
                review: review.clone(),
                thread: resolved.clone(),
            }),
            Ok(Reply::Done)
        );
        let workspace = setup.workspace.clone();
        let (store, scratch) = setup.finish();

        let reviews = Reviews::new(
            store,
            Git::find().expect("git"),
            scratch.0.join("data").join("states"),
        );
        let session = yaar_store::SessionId::new(session.as_str());
        for (thread, text, addressed) in [
            (&addressed, "done", true),
            (&waiting, "which one do you mean?", false),
            (&also_addressed, "done too", true),
        ] {
            reviews
                .agent_replied(
                    &session,
                    &ThreadReply {
                        thread: yaar_store::ThreadId::new(thread.as_str()),
                        text: text.to_owned(),
                        addressed,
                    },
                )
                .expect("keep the reply");
        }
        drop(reviews);

        let keeper = scratch.keeper();
        let window = scratch.window();
        Answered {
            _scratch: scratch,
            keeper,
            window,
            workspace,
            review,
            sent,
            addressed,
            waiting,
            resolved,
            also_addressed,
        }
    }

    fn review(&mut self) -> yaar_link::Review {
        self.window
            .reviews(&self.workspace)
            .reviews
            .into_iter()
            .find(|held| held.id == self.review)
            .expect("the review is kept")
    }
}

#[test]
fn each_thread_is_told_in_the_state_the_engines_rule_gives_it() {
    let mut world = Answered::new();
    let held = world.review();
    assert_eq!(state_of(&held, &world.sent), ThreadState::Sent);
    assert_eq!(state_of(&held, &world.addressed), ThreadState::Addressed);
    assert_eq!(
        state_of(&held, &world.waiting),
        ThreadState::WaitsForReviewer
    );
    assert_eq!(state_of(&held, &world.resolved), ThreadState::Resolved);

    let (fresh, _) = world.window.draft(&world.review, 6, 3, "a new thread");
    let (_, follow_up) = ids(7);
    assert_eq!(
        world.window.ask(Request::SaveDraft {
            review: world.review.clone(),
            thread: world.also_addressed.clone(),
            comment: follow_up,
            on: DraftOn::Thread,
            text: "and this too".to_owned(),
        }),
        Ok(Reply::Done)
    );
    let held = world.review();
    assert_eq!(state_of(&held, &fresh), ThreadState::Draft);
    assert_eq!(state_of(&held, &world.also_addressed), ThreadState::Draft);
    assert_eq!(state_of(&held, &world.addressed), ThreadState::Addressed);

    assert_eq!(
        world.window.ask(Request::ResolveThread {
            review: world.review.clone(),
            thread: world.also_addressed.clone(),
        }),
        Ok(Reply::Done)
    );
    assert_eq!(
        state_of(&world.review(), &world.also_addressed),
        ThreadState::Resolved
    );
    assert_eq!(
        world.window.ask(Request::ReopenThread {
            review: world.review.clone(),
            thread: world.resolved.clone(),
        }),
        Ok(Reply::Done)
    );
    assert_eq!(
        state_of(&world.review(), &world.resolved),
        ThreadState::Sent
    );
    world.window.stop(world.keeper);
}

#[test]
fn resolving_every_addressed_thread_resolves_those_and_no_other_and_tells_only_when_one_was() {
    let mut world = Answered::new();
    world.window.flush();
    let mark = world.window.facts.len();

    assert_eq!(
        world.window.ask(Request::ResolveAddressed {
            review: world.review.clone()
        }),
        Ok(Reply::Done)
    );
    world.window.flush();
    assert_eq!(
        world.window.reviews_changed_since(mark, &world.workspace),
        1
    );
    let held = world.review();
    assert_eq!(state_of(&held, &world.addressed), ThreadState::Resolved);
    assert_eq!(
        state_of(&held, &world.also_addressed),
        ThreadState::Resolved
    );
    assert_eq!(
        state_of(&held, &world.waiting),
        ThreadState::WaitsForReviewer
    );
    assert_eq!(state_of(&held, &world.sent), ThreadState::Sent);
    assert_eq!(state_of(&held, &world.resolved), ThreadState::Resolved);

    let mark = world.window.facts.len();
    assert_eq!(
        world.window.ask(Request::ResolveAddressed {
            review: world.review.clone()
        }),
        Ok(Reply::Done)
    );
    world.window.flush();
    assert_eq!(
        world.window.reviews_changed_since(mark, &world.workspace),
        0
    );
    let nobody = ReviewId::new("0".repeat(32));
    assert_eq!(
        world.window.ask(Request::ResolveAddressed {
            review: nobody.clone()
        }),
        Err(Refusal::UnknownReview { review: nobody })
    );
    world.window.stop(world.keeper);
}

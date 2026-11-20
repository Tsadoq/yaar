//! What the component tests of the terminal pane share: the app prepared as the Window
//! program prepares it, a window with the pane beside a second pane, and the Keeper's
//! end of the Window's connection, played by the test.
//!
//! A test file starts with `mod common;` and takes what it needs. The usual set-up is
//! [`open_panes`], which gives a [`Panes`] and the context to drive the window with, and
//! a workspace on screen, so that New Terminal has somewhere to open a terminal:
//!
//! ```ignore
//! let (mut panes, mut cx) = open_panes(cx);
//! let one = panes.open_terminal(&mut cx);          // presses New Terminal, answers it
//! panes.keeper.output(&one, b"hello");             // what the program writes
//! cx.simulate_input("x");                          // what the user types
//! assert_eq!(panes.keeper.input(&mut cx).1, b"x"); // what the program receives
//! ```
//!
//! The Keeper is the one seam, as in every test of the Window: the Link on a socket pair,
//! spoken with `yaar-link` and nothing else. Everything the Window asks and sends the
//! test reads from [`TestKeeper`]; everything a Keeper would tell it the test sends.

// Each test program takes what it needs of this.
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use gpui_kit::{
    AppContext, Bounds, Context, Entity, FocusHandle, InteractiveElement, IntoElement, Modifiers,
    MouseButton, Pixels, Point, Render, Size, Styled, TestAppContext, VisualTestContext, Window,
    WindowBounds, WindowOptions, div, px, size,
};
use yaar_frame::keeper::Attached;
use yaar_frame::{BuildPane, Frame, KeeperLink, Models, Pane, bind, controls};
use yaar_link::{
    CreditedFile, Head, Hello, History, Message, PROTOCOL_VERSION, Refusal, Reply, Repository,
    RepositoryId, Request, RequestId, ReviewId, Setting, TerminalId, TerminalInfo, TerminalSize,
    Workspace, WorkspaceAgents, WorkspaceId, read_message, write_message,
};
use yaar_terminal_pane::TerminalPane;

/// How long a test waits for something that has to happen. Reaching it fails the test.
pub const DEADLINE: Duration = Duration::from_secs(10);

/// How long a wait rests between two looks.
const PAUSE: Duration = Duration::from_millis(1);

/// The size of the window the panes are shown in.
pub const WINDOW: Size<Pixels> = size(px(1200.), px(700.));

/// Prepares the app as `yaar_window::init` does, minus the review pane: the toolkit, the
/// keys it binds for the whole window handed back to a program, then the keys of the
/// Frame and of the terminal pane, in that order.
///
/// Without the first two steps Tab, Shift-Tab and, off macOS, Ctrl-C and Ctrl-Shift-I
/// would reach a terminal in a test and be lost in the app, so every test that presses
/// keys prepares the app with this and never with `bind` alone.
pub fn init_as_the_app_does(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        yaar_frame::release_program_keys(cx);
        let mut table = controls();
        table.extend(yaar_terminal_pane::controls());
        bind(&table, cx);
    });
}

/// The id of a terminal made of one character repeated, as the Keeper's ids are long.
pub fn terminal(character: char) -> TerminalId {
    TerminalId::new(character.to_string().repeat(32))
}

/// The id of a workspace made of one digit.
pub fn workspace_id(digit: char) -> WorkspaceId {
    WorkspaceId::new(digit.to_string().repeat(32))
}

/// A repository `repository-a` with one worktree, the workspace `workspace_id('1')`.
pub fn one_repository() -> Repository {
    Repository {
        id: RepositoryId::new("a".repeat(32)),
        name: "repository-a".to_owned(),
        workspaces: vec![Workspace {
            id: workspace_id('1'),
            path: PathBuf::from("/gits/main"),
            head: Some(Head::Branch("main".to_owned())),
        }],
    }
}

/// A repository `repository-a` with two worktrees, the workspaces `workspace_id('1')` and
/// `workspace_id('2')`, in that order.
pub fn two_workspaces() -> Repository {
    let mut repository = one_repository();
    repository.workspaces.push(Workspace {
        id: workspace_id('2'),
        path: PathBuf::from("/gits/other"),
        head: Some(Head::Branch("other".to_owned())),
    });
    repository
}

/// What the test keeps of the pane the Frame builds: the pane and its own focus handle.
type Built = (Entity<TerminalPane>, FocusHandle);

/// What the Window asks at times a test does not choose, besides the resizes and the
/// changes, and the Keeper answers itself, so that the question never reaches the test as
/// its next request: the agents and the terminals at each connect, the workspace on screen
/// at each change of it, the history and the uncommitted changes of a pane. Each is
/// answered from what a test set, and counted.
#[derive(Default)]
struct Standing {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    agents: Vec<WorkspaceAgents>,
    /// What every `ListClis` is answered with: none until a test sets them.
    clis: Vec<yaar_link::AgentCli>,
    terminals: Vec<TerminalInfo>,
    /// By workspace; a workspace with none set has a history with no entry.
    histories: HashMap<WorkspaceId, History>,
    uncommitted: Vec<CreditedFile>,
    /// What every `GetSettings` is answered with; each `SetSetting` takes the place of the
    /// setting it is the same thing as, or comes last, as the Keeper keeps them.
    settings: Vec<Setting>,
    /// What each `SetSetting` named, in the order they were heard.
    settings_set: Vec<Setting>,
    /// What each `ShowWorkspace` named, in the order they were heard.
    shown: Vec<Option<WorkspaceId>>,
    /// The review the last `GetUncommitted` named.
    uncommitted_review: Option<ReviewId>,
    /// What every `ShowWorkspace` is refused with, as the Keeper of the contract does.
    show_refusal: Option<Refusal>,
    asked: Asked,
    agents_held: Held,
    terminals_held: Held,
    /// Every frame of input, in the order it was read.
    inputs: Vec<(TerminalId, Vec<u8>)>,
    /// How many stop frames were read.
    stops: usize,
}

/// How many times each question was heard.
#[derive(Default, Clone, Copy)]
struct Asked {
    agents: usize,
    terminals: usize,
    history: usize,
    uncommitted: usize,
    settings: usize,
}

/// The answers to one kind of question that a test keeps back: a Keeper that is slow.
/// Each is made when the question is heard, as a Keeper makes it, and sent when released.
#[derive(Default)]
struct Held {
    holding: bool,
    held: VecDeque<(RequestId, Reply)>,
}

/// What the reading thread does with a request it reads.
enum Heard {
    /// It is the test's to read.
    Elsewhere,
    /// Answer it at once.
    Answer(Result<Reply, Refusal>),
    /// Its answer is kept back.
    Kept,
}

impl Held {
    fn release(&mut self, count: usize) -> Vec<(RequestId, Reply)> {
        let count = count.min(self.held.len());
        self.held.drain(..count).collect()
    }

    fn offer(&mut self, id: RequestId, reply: Reply) -> Heard {
        if self.holding {
            self.held.push_back((id, reply));
            Heard::Kept
        } else {
            Heard::Answer(Ok(reply))
        }
    }
}

impl Standing {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn hear(&self, id: RequestId, request: &Request) -> Heard {
        let mut inner = self.lock();
        match request {
            Request::ListAgents => {
                inner.asked.agents += 1;
                let reply = Reply::Agents(inner.agents.clone());
                inner.agents_held.offer(id, reply)
            }
            // The Window asks once for each connection, at a moment the test does not
            // choose: answered here, as the agents are.
            Request::ListClis => Heard::Answer(Ok(Reply::Clis(inner.clis.clone()))),
            Request::ListTerminals => {
                inner.asked.terminals += 1;
                let reply = Reply::Terminals(inner.terminals.clone());
                inner.terminals_held.offer(id, reply)
            }
            Request::ShowWorkspace { workspace } => {
                inner.shown.push(workspace.clone());
                Heard::Answer(match inner.show_refusal.clone() {
                    Some(refusal) => Err(refusal),
                    None => Ok(Reply::Done),
                })
            }
            Request::GetHistory { workspace } => {
                inner.asked.history += 1;
                let history = inner.histories.get(workspace).cloned();
                Heard::Answer(Ok(Reply::History(history.unwrap_or_else(|| History {
                    workspace: workspace.clone(),
                    start: None,
                    entries: Vec::new(),
                }))))
            }
            Request::GetUncommitted { review, .. } => {
                inner.asked.uncommitted += 1;
                inner.uncommitted_review = review.clone();
                Heard::Answer(Ok(Reply::Uncommitted(inner.uncommitted.clone())))
            }
            Request::GetSettings => {
                inner.asked.settings += 1;
                Heard::Answer(Ok(Reply::Settings(inner.settings.clone())))
            }
            Request::SetSetting { setting } => {
                inner.settings_set.push(setting.clone());
                match inner
                    .settings
                    .iter_mut()
                    .find(|held| held.same_thing_as(setting))
                {
                    Some(held) => *held = setting.clone(),
                    None => inner.settings.push(setting.clone()),
                }
                Heard::Answer(Ok(Reply::Done))
            }
            _ => Heard::Elsewhere,
        }
    }
}

/// The Keeper, played by the test: the other end of the Window's connection.
///
/// A thread of its own reads what the Window sends, since the thread of the test has to
/// keep the app running while it waits. Of what it reads, the thread answers every
/// [`Request::ResizeTerminal`] with `Done` itself and keeps its size, so that the test
/// never has to, refuses the question for the changes of a workspace, which the models ask
/// whenever one comes on screen and no pane here shows, and answers by itself what the
/// models ask at times a test does not choose (the agents, the terminals, the workspace on
/// screen, a history, uncommitted changes); every other request waits for
/// [`TestKeeper::request`], every `Input` frame for [`TestKeeper::input`], and the stop
/// frame for [`TestKeeper::shutdown`]. The three are read apart, each in the order the
/// Window sent it.
pub struct TestKeeper {
    writer: Arc<Mutex<UnixStream>>,
    requests: Receiver<(RequestId, Request)>,
    inputs: Receiver<(TerminalId, Vec<u8>)>,
    stops: Receiver<()>,
    sizes: Arc<Mutex<Vec<(TerminalId, TerminalSize)>>>,
    standing: Arc<Standing>,
}

impl TestKeeper {
    /// Links `link` to a Keeper that speaks the Window's protocol.
    pub fn attach(link: &Entity<KeeperLink>, cx: &mut TestAppContext) -> Self {
        Self::attach_tapping(link, cx, |_, _| {})
    }

    /// Links `link` to a Keeper that speaks the Window's protocol, and hands `tap` the
    /// bytes of every `Input` frame as the reading thread reads it, before the frame is
    /// kept for [`TestKeeper::input`]. A test that wants the bytes somewhere else, as a
    /// pseudo-terminal a program reads, writes them there from `tap`.
    pub fn attach_tapping(
        link: &Entity<KeeperLink>,
        cx: &mut TestAppContext,
        tap: impl FnMut(&TerminalId, &[u8]) + Send + 'static,
    ) -> Self {
        Self::attach_staged(link, cx, |_| {}, tap)
    }

    /// [`TestKeeper::attach_tapping`], with what the Keeper knows set by `stage` before
    /// the Window is linked to it: the Window asks for the agents and the terminals the
    /// moment it is, so what the first answers hold has to be set before.
    pub fn attach_staged(
        link: &Entity<KeeperLink>,
        cx: &mut TestAppContext,
        stage: impl FnOnce(&TestKeeper),
        mut tap: impl FnMut(&TerminalId, &[u8]) + Send + 'static,
    ) -> Self {
        // The connection is read by a real thread, which the test executor cannot see.
        cx.executor().allow_parking();
        let (ours, theirs) = UnixStream::pair().expect("a socket pair");
        let mut reading = ours.try_clone().expect("a second handle to the socket");
        let writer = Arc::new(Mutex::new(ours));
        let sizes = Arc::new(Mutex::new(Vec::new()));
        let standing = Arc::new(Standing::default());
        let (tell_request, requests) = channel();
        let (tell_input, inputs) = channel();
        let (tell_stop, stops) = channel();
        std::thread::spawn({
            let writer = writer.clone();
            let sizes = sizes.clone();
            let standing = standing.clone();
            move || {
                while let Ok(message) = read_message(&mut reading) {
                    match message {
                        Message::Request {
                            id,
                            request: Request::ResizeTerminal { terminal, size },
                        } => {
                            sizes.lock().expect("the sizes").push((terminal, size));
                            let done = Message::Reply {
                                to: id,
                                reply: Ok(Reply::Done),
                            };
                            let mut writer = writer.lock().expect("the writer");
                            if write_message(&mut *writer, &done).is_err() {
                                break;
                            }
                        }
                        // The Window asks for the changes of the workspace on screen as
                        // soon as one is: no pane of these tests shows them, and this end
                        // says it keeps none, so that no test has to read the question.
                        Message::Request {
                            id,
                            request: Request::GetChanges { .. },
                        } => {
                            let none = Message::Reply {
                                to: id,
                                reply: Err(Refusal::Failed {
                                    reason: "this Keeper keeps no changes".to_owned(),
                                }),
                            };
                            let mut writer = writer.lock().expect("the writer");
                            if write_message(&mut *writer, &none).is_err() {
                                break;
                            }
                        }
                        Message::Request { id, request } => match standing.hear(id, &request) {
                            Heard::Elsewhere => {
                                if tell_request.send((id, request)).is_err() {
                                    break;
                                }
                            }
                            Heard::Answer(reply) => {
                                let answer = Message::Reply { to: id, reply };
                                let mut writer = writer.lock().expect("the writer");
                                if write_message(&mut *writer, &answer).is_err() {
                                    break;
                                }
                            }
                            Heard::Kept => {}
                        },
                        Message::Input { terminal, bytes } => {
                            tap(&terminal, &bytes);
                            // Kept before the test can read the frame, so that what a
                            // test finds kept is never later than what it read.
                            standing
                                .lock()
                                .inputs
                                .push((terminal.clone(), bytes.clone()));
                            if tell_input.send((terminal, bytes)).is_err() {
                                break;
                            }
                        }
                        Message::Shutdown => {
                            standing.lock().stops += 1;
                            if tell_stop.send(()).is_err() {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
        });
        let keeper = Hello {
            protocol: PROTOCOL_VERSION,
            pid: 1,
            app_version: "9.9.9".to_owned(),
        };
        let end = Self {
            writer,
            requests,
            inputs,
            stops,
            sizes,
            standing,
        };
        stage(&end);
        link.update(cx, |link, cx| {
            link.attached(
                Ok(Attached {
                    stream: theirs,
                    keeper,
                    started: false,
                }),
                cx,
            )
        });
        end
    }

    /// Waits, with the app running, for the next message on `channel`. Fails at the
    /// deadline, naming `wanted`.
    fn next<T>(&self, channel: &Receiver<T>, wanted: &str, cx: &mut TestAppContext) -> T {
        let deadline = Instant::now() + DEADLINE;
        loop {
            cx.run_until_parked();
            match channel.recv_timeout(PAUSE) {
                Ok(message) => return message,
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("the Window closed the connection, and {wanted} was expected")
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            assert!(Instant::now() < deadline, "the Window sent no {wanted}");
        }
    }

    /// The next request of the Window but a resize, which this end answers itself.
    pub fn request(&self, cx: &mut TestAppContext) -> (RequestId, Request) {
        self.next(&self.requests, "a request", cx)
    }

    /// The next frame of input the Window sent to a terminal: the terminal and its bytes.
    /// A frame holds what one key or one paste sent, up to 64 KiB. Fails when none comes.
    pub fn input(&self, cx: &mut TestAppContext) -> (TerminalId, Vec<u8>) {
        self.next(&self.inputs, "input", cx)
    }

    /// The input of `terminal` up to and including `ending`, in as many frames as the
    /// Window sent it in. A test that wants to know that a key sent nothing types text
    /// it knows after it and reads up to that: what came before is what the key sent.
    /// Fails when the input of another terminal comes first.
    pub fn input_until(
        &mut self,
        terminal: &TerminalId,
        ending: &[u8],
        cx: &mut TestAppContext,
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        while !bytes.ends_with(ending) {
            let (from, more) = self.input(cx);
            assert_eq!(&from, terminal, "input of another terminal came first");
            bytes.extend(more);
        }
        bytes.truncate(bytes.len() - ending.len());
        bytes
    }

    /// Whether no frame of input has come that nobody read.
    pub fn no_input_waits(&self) -> bool {
        self.inputs.try_recv().is_err()
    }

    /// The sizes the Window gave a terminal after it was opened, in the order it did,
    /// each answered with `Done`.
    pub fn resizes(&self, terminal: &TerminalId) -> Vec<TerminalSize> {
        self.sizes
            .lock()
            .expect("the sizes")
            .iter()
            .filter(|(id, _)| id == terminal)
            .map(|(_, size)| *size)
            .collect()
    }

    /// Sets the agents every `ListAgents` is answered with from now on. It tells the
    /// Window nothing: a test that wants it to read them tells `Fact::AgentsChanged`.
    pub fn set_agents(&self, agents: Vec<WorkspaceAgents>) {
        self.standing.lock().agents = agents;
    }

    /// Sets the agent CLIs every `ListClis` is answered with from now on: the Window asks
    /// when it connects, so a test sets them before.
    pub fn set_clis(&self, clis: Vec<yaar_link::AgentCli>) {
        self.standing.lock().clis = clis;
    }

    /// Sets the terminals every `ListTerminals` is answered with from now on.
    pub fn set_terminals(&self, terminals: Vec<TerminalInfo>) {
        self.standing.lock().terminals = terminals;
    }

    /// Sets the history every `GetHistory` of its workspace is answered with from now on.
    pub fn set_history(&self, history: History) {
        let mut inner = self.standing.lock();
        inner.histories.insert(history.workspace.clone(), history);
    }

    /// Sets the changes every `GetUncommitted` is answered with from now on, whatever
    /// workspace and review it names.
    pub fn set_uncommitted(&self, uncommitted: Vec<CreditedFile>) {
        self.standing.lock().uncommitted = uncommitted;
    }

    /// From now on refuses every `ShowWorkspace` with this, as the Keeper of the contract
    /// does; none to answer them again.
    pub fn refuse_show(&self, refusal: Option<Refusal>) {
        self.standing.lock().show_refusal = refusal;
    }

    /// What each `ShowWorkspace` the Window sent named, in the order it sent them.
    pub fn shown_told(&self) -> Vec<Option<WorkspaceId>> {
        self.standing.lock().shown.clone()
    }

    /// The workspace the last `ShowWorkspace` named; none before one was heard, and when
    /// the last named none.
    pub fn shown_workspace(&self) -> Option<WorkspaceId> {
        self.standing.lock().shown.last().cloned().flatten()
    }

    /// The review the last `GetUncommitted` named; none before one was heard, and when
    /// the last named none.
    pub fn uncommitted_review(&self) -> Option<ReviewId> {
        self.standing.lock().uncommitted_review.clone()
    }

    /// Sets the settings every `GetSettings` is answered with from now on. It tells the
    /// Window nothing: a test that wants it to hear one tells `Fact::SettingChanged`.
    pub fn set_settings(&self, settings: Vec<Setting>) {
        self.standing.lock().settings = settings;
    }

    /// What each `SetSetting` the Window sent named, in the order it sent them.
    pub fn settings_set(&self) -> Vec<Setting> {
        self.standing.lock().settings_set.clone()
    }

    /// How many times the Window asked for the settings so far.
    pub fn settings_asked(&self) -> usize {
        self.standing.lock().asked.settings
    }

    /// How many times the Window asked for the agents so far.
    pub fn agents_asked(&self) -> usize {
        self.standing.lock().asked.agents
    }

    /// How many times the Window asked for the terminals so far.
    pub fn terminals_asked(&self) -> usize {
        self.standing.lock().asked.terminals
    }

    /// How many times the Window asked for a history so far.
    pub fn history_asked(&self) -> usize {
        self.standing.lock().asked.history
    }

    /// How many times the Window asked for uncommitted changes so far.
    pub fn uncommitted_asked(&self) -> usize {
        self.standing.lock().asked.uncommitted
    }

    /// From now on keeps the questions for the agents unanswered until
    /// [`TestKeeper::release_agents`]: a Keeper that is slow. Each answer is made when the
    /// question is heard, from what is set then.
    pub fn hold_agents(&self) {
        self.standing.lock().agents_held.holding = true;
    }

    /// How many answers for the agents are kept back.
    pub fn held_agents(&self) -> usize {
        self.standing.lock().agents_held.held.len()
    }

    /// Sends the `count` oldest kept answers for the agents, in the order they were asked.
    pub fn release_agents(&self, count: usize) {
        let released = self.standing.lock().agents_held.release(count);
        self.send_replies(released);
    }

    /// As [`TestKeeper::hold_agents`], for the terminals.
    pub fn hold_terminals(&self) {
        self.standing.lock().terminals_held.holding = true;
    }

    /// How many answers for the terminals are kept back.
    pub fn held_terminals(&self) -> usize {
        self.standing.lock().terminals_held.held.len()
    }

    /// Sends the `count` oldest kept answers for the terminals, in the order they were
    /// asked.
    pub fn release_terminals(&self, count: usize) {
        let released = self.standing.lock().terminals_held.release(count);
        self.send_replies(released);
    }

    fn send_replies(&self, replies: Vec<(RequestId, Reply)>) {
        for (to, reply) in replies {
            self.reply(to, Ok(reply));
        }
    }

    /// How many stop frames the Window sent so far. A frame is counted before the test
    /// can read it, so a count of none is read after a barrier: something the Window sent
    /// after, which this end heard.
    pub fn stops_heard(&self) -> usize {
        self.standing.lock().stops
    }

    /// Waits, with the app running, for the stop frame the Window sends. Fails when none
    /// comes.
    pub fn shutdown(&self, cx: &mut TestAppContext) {
        self.next(&self.stops, "the stop frame", cx);
    }

    /// The frames of input the Window sent to `terminal` so far, in the order it sent
    /// them, whether or not the test read them with [`TestKeeper::input`].
    pub fn inputs(&self, terminal: &TerminalId) -> Vec<Vec<u8>> {
        self.standing
            .lock()
            .inputs
            .iter()
            .filter(|(from, _)| from == terminal)
            .map(|(_, bytes)| bytes.clone())
            .collect()
    }

    /// Answers a request.
    pub fn reply(&self, to: RequestId, reply: Result<Reply, Refusal>) {
        self.write(&Message::Reply { to, reply });
    }

    /// Answers the question for the list of workspaces, which has to be the next
    /// request, with these repositories.
    pub fn list(&self, listed: &[Repository], cx: &mut TestAppContext) {
        let (id, request) = self.request(cx);
        assert_eq!(request, Request::ListWorkspaces);
        self.reply(id, Ok(Reply::Workspaces(listed.to_vec())));
    }

    /// Answers the next request, which has to be for a terminal, with the id `terminal`.
    /// Gives what the request asked.
    pub fn open(
        &mut self,
        terminal: &TerminalId,
        cx: &mut TestAppContext,
    ) -> (WorkspaceId, TerminalSize) {
        self.open_running(terminal, yaar_link::Program::Shell, cx)
    }

    /// [`TestKeeper::open`] for a terminal that has to run `program`: a launcher's or a
    /// resume's.
    pub fn open_running(
        &mut self,
        terminal: &TerminalId,
        wanted: yaar_link::Program,
        cx: &mut TestAppContext,
    ) -> (WorkspaceId, TerminalSize) {
        let (id, request) = self.request(cx);
        let Request::OpenTerminal {
            workspace,
            program,
            size,
        } = request
        else {
            panic!("the Window sent {request:?}, and a terminal was asked for");
        };
        assert_eq!(program, wanted);
        self.reply(
            id,
            Ok(Reply::TerminalOpened {
                terminal: terminal.clone(),
            }),
        );
        (workspace, size)
    }

    /// Sends the Window what the program of a terminal wrote.
    pub fn output(&self, terminal: &TerminalId, bytes: &[u8]) {
        self.write(&Message::Output {
            terminal: terminal.clone(),
            bytes: bytes.to_vec(),
        });
    }

    /// Tells the Window a fact. What the fact says of the agents of a workspace is what
    /// every `ListAgents` is answered with from then on, as the Keeper's list holds what
    /// it told: the Window asks again for the list when it hears a change while an answer
    /// is on its way, and an answer older than the fact would take the fact's sessions
    /// away again.
    pub fn tell(&self, fact: yaar_link::Fact) {
        if let yaar_link::Fact::AgentsChanged(told) = &fact {
            let mut inner = self.standing.lock();
            match inner
                .agents
                .iter_mut()
                .find(|agents| agents.workspace == told.workspace)
            {
                Some(held) => *held = told.clone(),
                None => inner.agents.push(told.clone()),
            }
        }
        self.write(&Message::Fact(fact));
    }

    fn write(&self, message: &Message) {
        let mut writer = self.writer.lock().expect("the writer");
        write_message(&mut *writer, message).expect("write to the Window");
    }
}

/// A pane that is no terminal: it takes the focus on a mouse down, so that a test can
/// leave a terminal by clicking into it, as a Linux user does before pressing a key of
/// the app. It draws an empty element that fills it, under the selector
/// `elsewhere-pane`.
struct Elsewhere {
    focus: FocusHandle,
}

impl Render for Elsewhere {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .debug_selector(|| "elsewhere-pane".to_owned())
            .size_full()
            .track_focus(&self.focus)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.focus.focus(window, cx)),
            )
    }
}

/// The builder of the pane `elsewhere`, to put beside the terminal pane in a Frame.
pub fn elsewhere() -> BuildPane {
    Box::new(|_, _, cx| {
        let view = cx.new(|cx| Elsewhere {
            focus: cx.focus_handle(),
        });
        let focus = view.read(cx).focus.clone();
        Pane {
            name: "elsewhere",
            view: view.into(),
            focus,
        }
    })
}

/// A window with the terminal pane and the pane `elsewhere`, and the Keeper that plays
/// its other end.
pub struct Panes {
    /// The terminal pane, as the test reads it.
    pub pane: Entity<TerminalPane>,
    /// The focus handle of the terminal pane itself, the one the Frame moves the focus
    /// to when it moves it to the pane.
    pub pane_focus: FocusHandle,
    /// The root view of the window.
    pub frame: Entity<Frame>,
    /// The shared models of the window.
    pub models: Models,
    /// The Keeper.
    pub keeper: TestKeeper,
    /// The size each terminal was asked for at, as its request said.
    opened: HashMap<TerminalId, TerminalSize>,
}

/// Opens a window the way the Window program does, short of reaching a real Keeper, with
/// the terminal pane first so that it has the focus, and the pane `elsewhere` after it.
/// The Keeper lists one repository with one workspace, which comes on screen. The app is
/// prepared by [`init_as_the_app_does`]. `tap` is handed the bytes of every `Input` frame,
/// see [`TestKeeper::attach_tapping`].
pub fn open_panes_tapping(
    cx: &mut TestAppContext,
    tap: impl FnMut(&TerminalId, &[u8]) + Send + 'static,
) -> (Panes, VisualTestContext) {
    open_panes_listing(cx, &[one_repository()], tap)
}

/// [`open_panes_tapping`] with the Keeper listing `listed`: the first workspace of the
/// list comes on screen.
pub fn open_panes_listing(
    cx: &mut TestAppContext,
    listed: &[Repository],
    tap: impl FnMut(&TerminalId, &[u8]) + Send + 'static,
) -> (Panes, VisualTestContext) {
    open_panes_staged(cx, listed, |_| {}, tap)
}

/// [`open_panes_listing`], with what the Keeper knows set by `stage` before the window is
/// linked to it: see [`TestKeeper::attach_staged`].
pub fn open_panes_staged(
    cx: &mut TestAppContext,
    listed: &[Repository],
    stage: impl FnOnce(&TestKeeper),
    tap: impl FnMut(&TerminalId, &[u8]) + Send + 'static,
) -> (Panes, VisualTestContext) {
    init_as_the_app_does(cx);
    let link = cx.new(|_| KeeperLink::new());
    let models = cx.update(|cx| Models::new(link.clone(), cx));

    // The pane is built by the Window's own builder, wrapped to keep what the test reads.
    let built: Rc<RefCell<Option<Built>>> = Rc::default();
    let terminal_pane: BuildPane = {
        let build = yaar_terminal_pane::pane();
        let built = built.clone();
        Box::new(move |models, window, cx| {
            let pane = build(models, window, cx);
            let view = pane
                .view
                .clone()
                .downcast::<TerminalPane>()
                .unwrap_or_else(|_| panic!("the terminal pane builds a TerminalPane"));
            *built.borrow_mut() = Some((view, pane.focus.clone()));
            pane
        })
    };

    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: Point::default(),
            size: WINDOW,
        })),
        ..Default::default()
    };
    let (window, frame) = cx
        .update(|cx| {
            gpui_kit::open_window(options, cx, {
                let models = models.clone();
                move |window, cx| {
                    cx.new(|cx| {
                        Frame::with_panes(models, vec![terminal_pane, elsewhere()], window, cx)
                    })
                }
            })
        })
        .expect("open a test window");
    let mut cx = VisualTestContext::from_window(window, cx);
    // The window a user types in is the active one; GPUI tells no focus change to a
    // window that is not.
    cx.update(|window, _| window.activate_window());
    // Whatever opening the window sets off is done before a Keeper is there to hear it,
    // so the first thing the Keeper hears is the question for the list.
    cx.run_until_parked();

    let (pane, pane_focus) = built.take().expect("the Frame built the terminal pane");
    let keeper = TestKeeper::attach_staged(&link, &mut cx, stage, tap);
    keeper.list(listed, &mut cx);
    wait_until(&mut cx, "the workspace on screen", |cx| {
        models
            .workspaces
            .read_with(cx, |list, _| list.selected().is_some())
    });
    (
        Panes {
            pane,
            pane_focus,
            frame,
            models,
            keeper,
            opened: HashMap::new(),
        },
        cx,
    )
}

/// [`open_panes_tapping`] with no tap.
pub fn open_panes(cx: &mut TestAppContext) -> (Panes, VisualTestContext) {
    open_panes_tapping(cx, |_, _| {})
}

impl Panes {
    /// Where the focus is: the terminal view that has it, if one does.
    pub fn focused_terminal(&self, cx: &mut VisualTestContext) -> Option<TerminalId> {
        let pane = self.pane.clone();
        let ids = pane.read_with(cx, |pane, _| pane.terminals());
        ids.into_iter().find(|id| {
            let view = pane
                .read_with(cx, |pane, _| pane.terminal(id))
                .expect("a terminal of the pane");
            let focus = view.read_with(cx, |view, _| view.focus_handle());
            cx.update(|window, _| focus.is_focused(window))
        })
    }

    /// Opens a terminal as a user does: leaves the terminal with a click into the other
    /// pane, presses the key of New Terminal, and answers the question for it with `id`.
    /// Waits until the pane holds the terminal.
    pub fn open_terminal(&mut self, id: &TerminalId, cx: &mut VisualTestContext) {
        self.press_in_the_app::<yaar_terminal_pane::NewTerminal>(cx);
        self.answer_open(id, yaar_link::Program::Shell, cx);
    }

    /// Answers the next request, which has to ask for a terminal that runs `program`, with
    /// `id`, and waits until the pane holds the terminal. Gives the workspace and the size
    /// the request asked for.
    pub fn answer_open(
        &mut self,
        id: &TerminalId,
        program: yaar_link::Program,
        cx: &mut VisualTestContext,
    ) -> (WorkspaceId, TerminalSize) {
        let (workspace, size) = self.keeper.open_running(id, program, cx);
        self.opened.insert(id.clone(), size);
        let pane = self.pane.clone();
        let id = id.clone();
        wait_until(cx, "the terminal in the pane", move |cx| {
            pane.read_with(cx, |pane, _| pane.terminals().contains(&id))
        });
        (workspace, size)
    }

    /// Presses the key of a control of the terminal pane, after a click in the pane
    /// `elsewhere`: on Linux a key of the app does nothing while a terminal has the
    /// focus, and a user leaves it first. The control is found by its action.
    pub fn press_in_the_app<A: gpui_kit::Action + Default>(&mut self, cx: &mut VisualTestContext) {
        let key = key_of::<A>();
        self.click_elsewhere(cx);
        cx.simulate_keystrokes(key);
    }

    /// The grid a terminal has now: the size of its last `ResizeTerminal`, which the
    /// Keeper answered, else the size its `OpenTerminal` asked for.
    pub fn grid(&self, terminal: &TerminalId) -> TerminalSize {
        self.keeper
            .resizes(terminal)
            .last()
            .copied()
            .or_else(|| self.opened.get(terminal).copied())
            .unwrap_or_else(|| panic!("terminal {terminal} was not opened by this test"))
    }

    /// The middle of one cell of the terminal shown in the slot `slot`, counted from 0 at
    /// the top left, in the window's own pixels. The painter lays whole cells from the
    /// top left corner of the slot, so a cell is about the slot's width over the columns
    /// wide and its height over the rows high: the estimate cannot reach the next cell
    /// for a cell in the first five columns and lines, which is where a test aims.
    pub fn cell_centre(
        &self,
        terminal: &TerminalId,
        slot: usize,
        column: usize,
        line: usize,
        cx: &mut VisualTestContext,
    ) -> Point<Pixels> {
        assert!(
            column < 5 && line < 5,
            "the estimate holds in the first five cells"
        );
        let bounds = drawn(&format!("terminal-slot-{slot}"), cx)
            .unwrap_or_else(|| panic!("terminal-slot-{slot} is not drawn"));
        let grid = self.grid(terminal);
        let width = bounds.size.width / f32::from(grid.columns);
        let height = bounds.size.height / f32::from(grid.rows);
        bounds.origin + gpui_kit::point(width * (column as f32 + 0.5), height * (line as f32 + 0.5))
    }

    /// Clicks into the pane `elsewhere`, which takes the focus.
    pub fn click_elsewhere(&mut self, cx: &mut VisualTestContext) {
        click_in("elsewhere-pane", cx);
    }

    /// Clicks into the slot `slot` of the terminal pane, which gives its terminal the
    /// focus.
    pub fn click_slot(&mut self, slot: usize, cx: &mut VisualTestContext) {
        click_in(&format!("terminal-slot-{slot}"), cx);
    }
}

/// The key a control of the app has for this action, as the tables write it: a test
/// presses what the table has, so that it follows the table when a key changes.
pub fn key_of<A: gpui_kit::Action + Default>() -> &'static str {
    let name = A::default().name();
    let mut table = controls();
    table.extend(yaar_terminal_pane::controls());
    table
        .into_iter()
        .find(|control| (control.action)().name() == name)
        .unwrap_or_else(|| panic!("no control has the action {name}"))
        .keystroke
}

/// Where the element with this selector was drawn in the last frame, if it was.
pub fn drawn(selector: &str, cx: &mut VisualTestContext) -> Option<Bounds<Pixels>> {
    // The test context takes a text that lives as long as the program.
    cx.debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
}

/// Clicks the middle of the element with this selector, which has to be drawn.
pub fn click_in(selector: &str, cx: &mut VisualTestContext) {
    let at = drawn(selector, cx)
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
        .center();
    cx.simulate_click(at, Modifiers::none());
}

/// Keeps the app running until `holds` says yes. Fails at the deadline, naming `what`.
pub fn wait_until(
    cx: &mut TestAppContext,
    what: &str,
    mut holds: impl FnMut(&mut TestAppContext) -> bool,
) {
    let deadline = Instant::now() + DEADLINE;
    loop {
        cx.run_until_parked();
        if holds(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "waited in vain for {what}");
        std::thread::park_timeout(PAUSE);
    }
}

/// Keeps the app running until `holds` says yes: [`wait_until`] for what is read from the
/// window as drawn. Fails at the deadline, naming `what`.
pub fn wait_drawn(
    cx: &mut VisualTestContext,
    what: &str,
    mut holds: impl FnMut(&mut VisualTestContext) -> bool,
) {
    let deadline = Instant::now() + DEADLINE;
    loop {
        cx.run_until_parked();
        if holds(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "waited in vain for {what}");
        std::thread::park_timeout(PAUSE);
    }
}

/// The lines of the screen of a terminal of the pane, as its view reads them.
pub fn lines(pane: &Entity<TerminalPane>, id: &TerminalId, cx: &impl AppContext) -> Vec<String> {
    let view = pane
        .read_with(cx, |pane, _| pane.terminal(id))
        .expect("a terminal of the pane");
    view.read_with(cx, |view, _| view.lines())
}

/// The refusal of a request, for a test that plays a Keeper that says no.
pub fn refused(reason: &str) -> Refusal {
    Refusal::Failed {
        reason: reason.to_owned(),
    }
}

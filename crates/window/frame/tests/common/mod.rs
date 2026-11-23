//! The Keeper's end of the Window's connection, as a test plays it: one end of a socket
//! pair, spoken with `yaar-link` and nothing else. The Window's end is handed to the
//! [`KeeperLink`] the way a real connection is, so a test fills the models with what a
//! Keeper would send and reads what the Window really asks.

// Each test program takes what it needs of this.
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use gpui_kit::{
    Action, AppContext, Bounds, Entity, Pixels, Point, Size, Task, TestAppContext,
    VisualTestContext, WindowBounds, WindowOptions,
};
use yaar_frame::keeper::Attached;
use yaar_frame::{BuildPane, Frame, KeeperLink, Models, Sidebar, bind, controls};
use yaar_link::{
    CreditedFile, Fact, Head, Hello, History, Message, PROTOCOL_VERSION, Refusal, Reply,
    Repository, RepositoryId, Request, RequestId, ReviewId, Setting, TerminalId, TerminalInfo,
    Workspace, WorkspaceAgents, WorkspaceId, read_message, write_message,
};

/// How long a test waits for something that has to happen. Reaching it fails the test.
pub const DEADLINE: Duration = Duration::from_secs(10);

/// How long a wait rests between two looks.
const PAUSE: Duration = Duration::from_millis(1);

/// The id of a workspace, made of one digit.
pub fn id(digit: char) -> WorkspaceId {
    WorkspaceId::new(digit.to_string().repeat(32))
}

/// A worktree with this branch checked out.
pub fn workspace(digit: char, branch: &str) -> Workspace {
    Workspace {
        id: id(digit),
        path: PathBuf::from(format!("/gits/{branch}")),
        head: Some(Head::Branch(branch.to_owned())),
    }
}

/// A repository with these worktrees, named `repository-<letter>`.
pub fn repository(letter: char, workspaces: Vec<Workspace>) -> Repository {
    Repository {
        id: RepositoryId::new(letter.to_string().repeat(32)),
        name: format!("repository-{letter}"),
        workspaces,
    }
}

/// The key the Frame's table of controls gives an action. A test presses the key the
/// table has, so that it follows the table when a key changes.
pub fn key_of<A: Action + Default>() -> &'static str {
    let name = A::default().name();
    controls()
        .into_iter()
        .find(|control| (control.action)().name() == name)
        .expect("the action has a row in the table")
        .keystroke
}

/// The key the Frame's table gives an action, as a user presses it on the system the app
/// is built for: `secondary` written out.
pub fn pressed<A: Action + Default>() -> String {
    let name = A::default().name();
    controls()
        .into_iter()
        .find(|control| (control.action)().name() == name)
        .expect("the action has a row in the table")
        .key()
}

/// What the Window asks at times a test does not choose, and this end answers itself, so
/// that the question never reaches the test as its next request: the agents and the
/// terminals at each connect, the workspace on screen at each change of it, the history
/// and the uncommitted changes of a pane. Each is answered from what a test set, and
/// counted.
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
    /// What every `SetSetting` is refused with, when a test says so: the setting is then
    /// not kept.
    setting_refusal: Option<Refusal>,
    /// What each `ShowWorkspace` named, in the order they were heard.
    shown: Vec<Option<WorkspaceId>>,
    /// The review the last `GetUncommitted` named.
    uncommitted_review: Option<ReviewId>,
    /// What every `ShowWorkspace` is refused with, as the Keeper of the contract does.
    show_refusal: Option<Refusal>,
    asked: Asked,
    agents_held: Held,
    terminals_held: Held,
    settings_held: Held,
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

/// What this end does with a request it reads.
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
                if let Some(refusal) = inner.setting_refusal.clone() {
                    return Heard::Answer(Err(refusal));
                }
                match inner
                    .settings
                    .iter_mut()
                    .find(|held| held.same_thing_as(setting))
                {
                    Some(held) => *held = setting.clone(),
                    None => inner.settings.push(setting.clone()),
                }
                inner.settings_held.offer(id, Reply::Done)
            }
            _ => Heard::Elsewhere,
        }
    }
}

/// Writes a message on the one stream of this end. A lock keeps the frames whole, since
/// the thread that answers and the test both write.
fn write(stream: &Mutex<UnixStream>, message: &Message) -> std::io::Result<()> {
    let mut stream = stream.lock().unwrap_or_else(PoisonError::into_inner);
    write_message(&mut *stream, message).map_err(|error| std::io::Error::other(error.to_string()))
}

/// The Keeper, played by the test.
pub struct KeeperEnd {
    stream: Arc<Mutex<UnixStream>>,
    // What the Window sent, read by a thread of this end's own: the thread of the test
    // has to keep the app running while it waits. What this end answers by itself is not
    // in it.
    heard: Receiver<Message>,
    standing: Arc<Standing>,
}

impl KeeperEnd {
    /// Links `link` to a Keeper that speaks the Window's protocol.
    pub fn attach(link: &Entity<KeeperLink>, cx: &mut TestAppContext) -> Self {
        Self::attach_speaking(PROTOCOL_VERSION, link, cx)
    }

    /// Links `link` to a Keeper that speaks this version of the protocol.
    pub fn attach_speaking(
        protocol: u16,
        link: &Entity<KeeperLink>,
        cx: &mut TestAppContext,
    ) -> Self {
        Self::attach_saying(
            Hello {
                protocol,
                pid: 1,
                app_version: "9.9.9".to_owned(),
            },
            link,
            cx,
        )
    }

    /// Links `link` to a Keeper that says this hello: the protocol it speaks and the
    /// version of the app it belongs to.
    pub fn attach_saying(
        keeper: Hello,
        link: &Entity<KeeperLink>,
        cx: &mut TestAppContext,
    ) -> Self {
        Self::attach_staged(keeper, link, cx, |_| {})
    }

    /// [`KeeperEnd::attach_saying`], with what the Keeper knows set by `stage` before the
    /// Window is linked to it: the Window asks for the agents and the terminals the
    /// moment it is, so what the first answers hold has to be set before.
    pub fn attach_staged(
        keeper: Hello,
        link: &Entity<KeeperLink>,
        cx: &mut TestAppContext,
        stage: impl FnOnce(&KeeperEnd),
    ) -> Self {
        // The connection is read by a real thread, which the test executor cannot see.
        cx.executor().allow_parking();
        let (ours, theirs) = UnixStream::pair().expect("a socket pair");
        let mut reading = ours.try_clone().expect("a second handle to the socket");
        let stream = Arc::new(Mutex::new(ours));
        let standing = Arc::new(Standing::default());
        let (tell, heard) = channel();
        {
            let (stream, standing) = (stream.clone(), standing.clone());
            std::thread::spawn(move || {
                while let Ok(message) = read_message(&mut reading) {
                    match message {
                        Message::Request { id, request } => match standing.hear(id, &request) {
                            Heard::Elsewhere => {
                                if tell.send(Message::Request { id, request }).is_err() {
                                    break;
                                }
                            }
                            // A Window that is gone no longer reads: the next read ends
                            // this thread.
                            Heard::Answer(reply) => {
                                let _ = write(&stream, &Message::Reply { to: id, reply });
                            }
                            Heard::Kept => {}
                        },
                        message => {
                            // Counted before the test can read the frame, so that what a
                            // test finds counted is never later than what it read.
                            match &message {
                                Message::Input { terminal, bytes } => standing
                                    .lock()
                                    .inputs
                                    .push((terminal.clone(), bytes.clone())),
                                Message::Shutdown => standing.lock().stops += 1,
                                _ => {}
                            }
                            if tell.send(message).is_err() {
                                break;
                            }
                        }
                    }
                }
            });
        }
        let end = Self {
            stream,
            heard,
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

    /// Links `link` to a Keeper that never reads: the connection is open, and what the
    /// Window writes to it stays where it is. Gives the Keeper's end, which keeps the
    /// connection open for as long as it lives.
    pub fn attach_deaf(link: &Entity<KeeperLink>, cx: &mut TestAppContext) -> UnixStream {
        cx.executor().allow_parking();
        let (ours, theirs) = UnixStream::pair().expect("a socket pair");
        let keeper = Hello {
            protocol: PROTOCOL_VERSION,
            pid: 1,
            app_version: "9.9.9".to_owned(),
        };
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
        ours
    }

    /// The next message of the Window, waiting for it with the app running. Fails when
    /// none comes, or when the Window closes the connection.
    fn message(&mut self, wanted: &str, cx: &mut TestAppContext) -> Message {
        let deadline = Instant::now() + DEADLINE;
        loop {
            cx.run_until_parked();
            match self.heard.recv_timeout(PAUSE) {
                Ok(message) => return message,
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("the Window closed the connection, and {wanted} was expected")
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            assert!(Instant::now() < deadline, "the Window sent no {wanted}");
        }
    }

    /// The next request of the Window. Fails when none comes, when the Window sends
    /// anything else, or when it closes the connection.
    pub fn request(&mut self, cx: &mut TestAppContext) -> (RequestId, Request) {
        match self.message("a request", cx) {
            Message::Request { id, request } => (id, request),
            other => panic!("the Window sent {other:?}, which is no request"),
        }
    }

    /// The next input the Window sends to a terminal: the terminal and the bytes of one
    /// frame. Fails when none comes, when the Window sends anything else, or when it
    /// closes the connection.
    pub fn input(&mut self, cx: &mut TestAppContext) -> (TerminalId, Vec<u8>) {
        match self.message("input", cx) {
            Message::Input { terminal, bytes } => (terminal, bytes),
            other => panic!("the Window sent {other:?}, which is no input"),
        }
    }

    /// Sends the Window the output of a terminal's program.
    pub fn output(&mut self, terminal: &TerminalId, bytes: &[u8]) {
        let message = Message::Output {
            terminal: terminal.clone(),
            bytes: bytes.to_vec(),
        };
        write(&self.stream, &message).expect("write output");
    }

    /// Sends the Window a message of any kind.
    pub fn send(&mut self, message: &Message) {
        write(&self.stream, message).expect("write a message");
    }

    /// The next request of the Window, which has to be this one; gives its id.
    pub fn expect(&mut self, wanted: Request, cx: &mut TestAppContext) -> RequestId {
        let (id, request) = self.request(cx);
        assert_eq!(request, wanted);
        id
    }

    /// Answers the question for the list of workspaces, which has to be the next request,
    /// with these repositories.
    pub fn list(&mut self, listed: &[Repository], cx: &mut TestAppContext) {
        let asked = self.expect(Request::ListWorkspaces, cx);
        self.reply(asked, Ok(Reply::Workspaces(listed.to_vec())));
    }

    /// Answers a request.
    pub fn reply(&mut self, to: RequestId, reply: Result<Reply, Refusal>) {
        write(&self.stream, &Message::Reply { to, reply }).expect("write a reply");
    }

    /// Tells the Window a fact.
    pub fn tell(&mut self, fact: Fact) {
        write(&self.stream, &Message::Fact(fact)).expect("write a fact");
    }

    /// Sends bytes as they are.
    pub fn send_raw(&mut self, bytes: &[u8]) {
        use std::io::Write;
        let mut stream = self.stream.lock().expect("the stream");
        stream.write_all(bytes).expect("write bytes");
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

    /// From now on refuses every `SetSetting` with this, and keeps nothing of it; none to
    /// keep and answer them again. Each is still listed in [`KeeperEnd::settings_set`].
    pub fn refuse_settings(&self, refusal: Option<Refusal>) {
        self.standing.lock().setting_refusal = refusal;
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
    /// [`KeeperEnd::release_agents`]: a Keeper that is slow. Each answer is made when the
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

    /// As [`KeeperEnd::hold_agents`], for the terminals.
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

    /// From now on keeps the answer to each `SetSetting` unanswered until
    /// [`KeeperEnd::release_settings`]: a Keeper that has kept the setting and is slow to say
    /// so. The setting is kept, and listed in [`KeeperEnd::settings_set`], when it is heard.
    pub fn hold_settings(&self) {
        self.standing.lock().settings_held.holding = true;
    }

    /// How many answers to a `SetSetting` are kept back.
    pub fn held_settings(&self) -> usize {
        self.standing.lock().settings_held.held.len()
    }

    /// Sends the `count` oldest kept answers to a `SetSetting`, in the order they were asked.
    pub fn release_settings(&self, count: usize) {
        let released = self.standing.lock().settings_held.release(count);
        self.send_replies(released);
    }

    fn send_replies(&self, replies: Vec<(RequestId, Reply)>) {
        for (to, reply) in replies {
            write(
                &self.stream,
                &Message::Reply {
                    to,
                    reply: Ok(reply),
                },
            )
            .expect("write a reply");
        }
    }

    /// How many stop frames the Window sent so far. A frame is counted before the test
    /// can read it, so a count of none is read after a barrier: something the Window sent
    /// after, which this end heard.
    pub fn stops_heard(&self) -> usize {
        self.standing.lock().stops
    }

    /// The next message of the Window, which has to be the stop frame.
    pub fn shutdown(&mut self, cx: &mut TestAppContext) {
        match self.message("the stop frame", cx) {
            Message::Shutdown => {}
            other => panic!("the Window sent {other:?}, and the stop frame was expected"),
        }
    }

    /// The frames of input the Window sent to `terminal` so far, in the order it sent
    /// them, whether or not the test read them with [`KeeperEnd::input`].
    pub fn inputs(&self, terminal: &TerminalId) -> Vec<Vec<u8>> {
        self.standing
            .lock()
            .inputs
            .iter()
            .filter(|(from, _)| from == terminal)
            .map(|(_, bytes)| bytes.clone())
            .collect()
    }

    /// Closes the connection, as a Keeper that exits does.
    pub fn close(self) {
        self.stream
            .lock()
            .expect("the stream")
            .shutdown(Shutdown::Both)
            .expect("close the socket");
    }

    /// Everything the Window still sends, up to the moment it closes the connection.
    /// Fails when the Window keeps it open.
    pub fn rest(self, cx: &mut TestAppContext) -> Vec<Message> {
        let deadline = Instant::now() + DEADLINE;
        let mut rest = Vec::new();
        loop {
            cx.run_until_parked();
            match self.heard.recv_timeout(PAUSE) {
                Ok(message) => rest.push(message),
                Err(RecvTimeoutError::Disconnected) => return rest,
                Err(RecvTimeoutError::Timeout) => {}
            }
            assert!(
                Instant::now() < deadline,
                "the Window did not close the connection"
            );
        }
    }
}

/// Keeps the app running until `holds` says yes. Fails at the deadline, naming `what`.
pub fn wait_until(
    cx: &mut TestAppContext,
    what: &str,
    holds: impl FnMut(&mut TestAppContext) -> bool,
) {
    wait_until_within(DEADLINE, cx, what, holds);
}

/// Keeps the app running until `holds` says yes, for at most `within`. Fails at the
/// deadline, naming `what`. For what the Window itself waits for before it acts.
pub fn wait_until_within(
    within: Duration,
    cx: &mut TestAppContext,
    what: &str,
    mut holds: impl FnMut(&mut TestAppContext) -> bool,
) {
    let deadline = Instant::now() + within;
    loop {
        cx.run_until_parked();
        if holds(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "waited in vain for {what}");
        std::thread::park_timeout(PAUSE);
    }
}

/// Waits for what a task resolves to.
pub fn answer<T: 'static>(task: Task<T>, cx: &mut TestAppContext) -> T {
    let slot = Rc::new(RefCell::new(None));
    cx.foreground_executor()
        .spawn({
            let slot = slot.clone();
            async move {
                let answer = task.await;
                *slot.borrow_mut() = Some(answer);
            }
        })
        .detach();
    wait_until(cx, "an answer", |_| slot.borrow().is_some());
    slot.take().expect("the answer")
}

/// A window of the app on the test platform, with the Keeper the test plays.
pub struct Shown {
    /// The root view of the window.
    pub frame: Entity<Frame>,
    /// Its sidebar.
    pub sidebar: Entity<Sidebar>,
    /// The shared models of the window.
    pub models: Models,
    /// The other end of the window's connection.
    pub keeper: KeeperEnd,
}

/// Opens a window of this size the way the Window program does, short of reaching a real
/// Keeper: the toolkit prepared, the keys of the Frame bound from its table, the Frame
/// with its sidebar and these panes as the root view. Then links it to a Keeper the test
/// plays, which is asked for the list of workspaces first.
pub fn open_frame(
    window_size: Size<Pixels>,
    panes: Vec<BuildPane>,
    cx: &mut TestAppContext,
) -> (Shown, VisualTestContext) {
    let keeper = Hello {
        protocol: PROTOCOL_VERSION,
        pid: 1,
        app_version: "9.9.9".to_owned(),
    };
    open_frame_with_keeper(keeper, window_size, panes, cx)
}

/// [`open_frame`], with a Keeper that says this hello.
pub fn open_frame_with_keeper(
    keeper: Hello,
    window_size: Size<Pixels>,
    panes: Vec<BuildPane>,
    cx: &mut TestAppContext,
) -> (Shown, VisualTestContext) {
    open_frame_staged(keeper, window_size, panes, cx, |_| {})
}

/// [`open_frame_with_keeper`], with what the Keeper knows set by `stage` before the window
/// is linked to it: see [`KeeperEnd::attach_staged`].
pub fn open_frame_staged(
    keeper: Hello,
    window_size: Size<Pixels>,
    panes: Vec<BuildPane>,
    cx: &mut TestAppContext,
    stage: impl FnOnce(&KeeperEnd),
) -> (Shown, VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        bind(&controls(), cx);
    });
    let link = cx.new(|_| KeeperLink::new());
    let models = cx.update(|cx| Models::new(link.clone(), cx));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: Point::default(),
            size: window_size,
        })),
        ..Default::default()
    };
    let (window, frame) = cx
        .update(|cx| {
            gpui_kit::open_window(options, cx, {
                let models = models.clone();
                move |window, cx| cx.new(|cx| Frame::with_panes(models, panes, window, cx))
            })
        })
        .expect("open a test window");
    let mut cx = VisualTestContext::from_window(window, cx);
    // Whatever opening the window sets off is done before a Keeper is there to hear it,
    // so the first thing the Keeper hears is the question for the list.
    cx.run_until_parked();
    let sidebar = frame.read_with(&cx, |frame, _| {
        frame
            .sidebar()
            .expect("a Frame with panes has a sidebar")
            .clone()
    });
    let keeper = KeeperEnd::attach_staged(keeper, &link, &mut cx, stage);
    (
        Shown {
            frame,
            sidebar,
            models,
            keeper,
        },
        cx,
    )
}

/// Where the element with this selector was drawn in the last frame, if it was.
pub fn drawn(selector: &str, cx: &mut VisualTestContext) -> Option<Bounds<Pixels>> {
    // The test context takes a text that lives as long as the program.
    cx.debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
}

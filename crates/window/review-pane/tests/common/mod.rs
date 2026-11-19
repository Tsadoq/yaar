//! What the tests of the pane share: a window that holds the pane alone, and the Keeper's
//! end of the Window's connection, played by the test on one end of a socket pair with
//! `yaar-link` and nothing else. The models are filled by what a Keeper would send, and
//! the test reads what the Window really asks.

// Each test program takes what it needs of this.
#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use gpui_kit::{
    AnyView, AppContext, Bounds, Context, Entity, FocusHandle, IntoElement, ParentElement, Pixels,
    Point, Render, Size, Styled, TestAppContext, VisualTestContext, Window, WindowBounds,
    WindowOptions, div, px, size,
};
use yaar_frame::keeper::Attached;
use yaar_frame::{KeeperLink, Models, bind};
use yaar_link::{
    Base, BaseSource, ChangedFile, Changes, Commit, Comparison, Content, CreditedFile, Fact,
    FileSides, FileStatus, Head, Hello, History, Message, PROTOCOL_VERSION, Refusal, Reply,
    Repository, RepositoryId, Request, RequestId, Review, ReviewId, Session, SessionId,
    SessionState, Setting, Step, TerminalId, TerminalInfo, Turn, TurnId, Workspace,
    WorkspaceAgents, WorkspaceId, WorkspaceReviews, read_message, write_message,
};
use yaar_review_pane::{PaneRow, ReviewPane, Unshown};

/// How long a test waits for something that has to happen. Reaching it fails the test.
pub const DEADLINE: Duration = Duration::from_secs(10);

/// How long a wait rests between two looks.
const PAUSE: Duration = Duration::from_millis(1);

/// The size of the window the pane is tested in: small, so that a diff of some length
/// does not fit and the pane has to scroll.
pub const WINDOW: Size<Pixels> = size(px(900.), px(420.));

/// The full id of a commit, as the Link carries it.
pub const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

/// What the Keeper end answers to `GetReviews` by itself, so that the reviews the pane
/// asks for at times a test does not choose never reach the test as its next request.
#[derive(Default)]
struct Reviews {
    /// What a test set, by workspace; a workspace with nothing set has one empty review
    /// of no agent, [`no_agent_review`].
    set: Mutex<HashMap<WorkspaceId, WorkspaceReviews>>,
    /// How many `GetReviews` were heard, answered or held.
    heard: AtomicUsize,
    /// Whether the answers wait: the requests are kept until [`KeeperEnd::release_reviews`].
    holding: AtomicBool,
    /// The requests kept while `holding`.
    held: Mutex<Vec<(RequestId, WorkspaceId)>>,
    /// What every `GetReviews` is refused with, when a test says so.
    refusal: Mutex<Option<Refusal>>,
}

impl Reviews {
    /// What a `GetReviews` of `workspace` is answered with now.
    fn answer(&self, workspace: &WorkspaceId) -> Result<Reply, Refusal> {
        if let Some(refusal) = self.refusal.lock().expect("not poisoned").clone() {
            return Err(refusal);
        }
        Ok(Reply::Reviews(self.reviews_of(workspace)))
    }

    /// The reviews of `workspace` as a test set them.
    fn reviews_of(&self, workspace: &WorkspaceId) -> WorkspaceReviews {
        self.set
            .lock()
            .expect("the reviews are not poisoned")
            .get(workspace)
            .cloned()
            .unwrap_or_else(|| WorkspaceReviews {
                workspace: workspace.clone(),
                reviews: vec![no_agent_review()],
                viewed: Vec::new(),
            })
    }
}

/// Writes a message on the one stream of this end. A lock keeps the frames whole, since
/// the thread that answers the reviews and the test both write.
fn write(stream: &Mutex<UnixStream>, message: &Message) {
    let mut stream = stream.lock().expect("the stream is not poisoned");
    write_message(&mut *stream, message).expect("write a message");
}

/// What the Window asks at times a test does not choose, besides the reviews, and this end
/// answers itself, so that the question never reaches the test as its next request: the
/// agents, the terminals and the settings at each connect, the workspace on screen at each
/// change of it, the history and the uncommitted changes of the pane, a setting the user
/// set, and a turn with its steps. Each is answered from what a test set, and counted.
#[derive(Default)]
struct Standing {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    agents: Vec<WorkspaceAgents>,
    terminals: Vec<TerminalInfo>,
    /// By workspace; a workspace with none set has a history that starts where the last
    /// changes answered for it are counted from, and holds no entry.
    histories: HashMap<WorkspaceId, History>,
    /// What the last `Changes` answered for each workspace is counted from.
    counted_from: HashMap<WorkspaceId, Option<String>>,
    uncommitted: Vec<CreditedFile>,
    /// What every `GetSettings` is answered with; each `SetSetting` takes the place of the
    /// setting it is the same thing as, or comes last, as the Keeper keeps them.
    settings: Vec<Setting>,
    /// What each `SetSetting` named, in the order they were heard.
    settings_set: Vec<Setting>,
    /// The turns `GetTurn` is answered with, by id; a turn with none set is refused as
    /// unknown.
    turns: HashMap<TurnId, Turn>,
    /// What each `GetTurn` named, in the order they were heard.
    turns_asked: Vec<TurnId>,
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
                let start = inner.counted_from.get(workspace).cloned().flatten();
                Heard::Answer(Ok(Reply::History(history.unwrap_or_else(|| History {
                    workspace: workspace.clone(),
                    start,
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
            Request::GetTurn { turn } => {
                inner.turns_asked.push(turn.clone());
                Heard::Answer(match inner.turns.get(turn) {
                    Some(found) => Ok(Reply::Turn(found.clone())),
                    None => Err(Refusal::UnknownTurn { turn: turn.clone() }),
                })
            }
            _ => Heard::Elsewhere,
        }
    }
}

/// The Keeper, played by the test.
pub struct KeeperEnd {
    stream: Arc<Mutex<UnixStream>>,
    // What the Window sent, read by a thread of this end's own: the thread of the test
    // has to keep the app running while it waits. What this end answers by itself is not
    // in it.
    heard: Receiver<Message>,
    reviews: Arc<Reviews>,
    standing: Arc<Standing>,
}

impl KeeperEnd {
    /// Links `link` to a Keeper that speaks the Window's protocol.
    pub fn attach(link: &Entity<KeeperLink>, cx: &mut TestAppContext) -> Self {
        Self::attach_staged(link, cx, |_| {})
    }

    /// [`KeeperEnd::attach`], with what the Keeper knows set by `stage` before the Window
    /// is linked to it: the Window asks for the agents and the settings the moment it is,
    /// so what the first answers hold has to be set before.
    pub fn attach_staged(
        link: &Entity<KeeperLink>,
        cx: &mut TestAppContext,
        stage: impl FnOnce(&KeeperEnd),
    ) -> Self {
        // The connection is read by a real thread, which the test executor cannot see.
        cx.executor().allow_parking();
        let (ours, theirs) = UnixStream::pair().expect("a socket pair");
        let mut reading = ours.try_clone().expect("a second handle to the socket");
        let stream = Arc::new(Mutex::new(ours));
        let reviews = Arc::new(Reviews::default());
        let standing = Arc::new(Standing::default());
        let (tell, heard) = channel();
        {
            let (stream, reviews, standing) = (stream.clone(), reviews.clone(), standing.clone());
            std::thread::spawn(move || {
                while let Ok(message) = read_message(&mut reading) {
                    match message {
                        // The reviews are asked for at times the test does not choose:
                        // they are answered here, and the test never hears of them.
                        Message::Request {
                            id,
                            request: Request::GetReviews { workspace },
                        } => {
                            reviews.heard.fetch_add(1, Ordering::SeqCst);
                            let mut held = reviews.held.lock().expect("not poisoned");
                            if reviews.holding.load(Ordering::SeqCst) {
                                held.push((id, workspace));
                            } else {
                                drop(held);
                                let answer = reviews.answer(&workspace);
                                write(
                                    &stream,
                                    &Message::Reply {
                                        to: id,
                                        reply: answer,
                                    },
                                );
                            }
                        }
                        Message::Request { id, request } => match standing.hear(id, &request) {
                            Heard::Elsewhere => {
                                if tell.send(Message::Request { id, request }).is_err() {
                                    break;
                                }
                            }
                            // A Window that is gone no longer reads: the next read ends
                            // this thread.
                            Heard::Answer(reply) => {
                                let mut stream =
                                    stream.lock().unwrap_or_else(PoisonError::into_inner);
                                let _ =
                                    write_message(&mut *stream, &Message::Reply { to: id, reply });
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
            reviews,
            standing,
        };
        stage(&end);
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
        end
    }

    /// The next request of the Window that is not for the reviews, which this end answers
    /// itself. Fails when none comes, when the Window sends anything else, or when it
    /// closes the connection.
    pub fn request(&mut self, cx: &mut TestAppContext) -> (RequestId, Request) {
        let deadline = Instant::now() + DEADLINE;
        loop {
            cx.run_until_parked();
            match self.heard.recv_timeout(PAUSE) {
                Ok(Message::Request { id, request }) => return (id, request),
                Ok(other) => panic!("the Window sent {other:?}, which is no request"),
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("the Window closed the connection, and a request was expected")
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            assert!(Instant::now() < deadline, "the Window asked nothing");
        }
    }

    /// The next request of the Window, which has to be this one; gives its id.
    pub fn expect(&mut self, wanted: Request, cx: &mut TestAppContext) -> RequestId {
        let (id, request) = self.request(cx);
        assert_eq!(request, wanted);
        id
    }

    /// Answers a request.
    pub fn reply(&mut self, to: RequestId, reply: Result<Reply, Refusal>) {
        // What the history of a workspace starts from by default, before it is asked for.
        if let Ok(Reply::Changes(changes)) = &reply {
            self.standing
                .lock()
                .counted_from
                .insert(changes.workspace.clone(), changes.counted_from.clone());
        }
        write(&self.stream, &Message::Reply { to, reply });
    }

    /// Tells the Window a fact.
    pub fn tell(&mut self, fact: Fact) {
        write(&self.stream, &Message::Fact(fact));
    }

    /// Sets the agents every `ListAgents` is answered with from now on. It tells the
    /// Window nothing: a test that wants it to read them tells `Fact::AgentsChanged`.
    pub fn set_agents(&self, agents: Vec<WorkspaceAgents>) {
        self.standing.lock().agents = agents;
    }

    /// Sets the agents of one workspace as the Keeper holds them, and tells the Window they
    /// changed, as the Keeper does when a session starts or its state changes.
    pub fn tell_agents(&mut self, agents: WorkspaceAgents) {
        self.set_agents(vec![agents.clone()]);
        self.tell(Fact::AgentsChanged(agents));
    }

    /// Sets the terminals every `ListTerminals` is answered with from now on.
    pub fn set_terminals(&self, terminals: Vec<TerminalInfo>) {
        self.standing.lock().terminals = terminals;
    }

    /// Sets the history every `GetHistory` of its workspace is answered with from now on.
    /// A workspace with none set has a history with no entry, that starts where the last
    /// changes this end answered for it are counted from.
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

    /// Sets the turn every `GetTurn` of its id is answered with from now on, its steps so
    /// far among it. A turn with none set is refused as unknown.
    pub fn set_turn(&self, turn: Turn) {
        self.standing.lock().turns.insert(turn.id.clone(), turn);
    }

    /// What each `GetTurn` the Window sent named, in the order it sent them.
    pub fn turns_asked(&self) -> Vec<TurnId> {
        self.standing.lock().turns_asked.clone()
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

    fn send_replies(&self, replies: Vec<(RequestId, Reply)>) {
        for (to, reply) in replies {
            write(
                &self.stream,
                &Message::Reply {
                    to,
                    reply: Ok(reply),
                },
            );
        }
    }

    /// How many stop frames the Window sent so far. A frame is counted before the test
    /// can read it, so a count of none is read after a barrier: something the Window sent
    /// after, which this end heard.
    pub fn stops_heard(&self) -> usize {
        self.standing.lock().stops
    }

    /// The next message of the Window, which has to be the stop frame. Fails when none
    /// comes, or when the Window sends anything else.
    pub fn shutdown(&mut self, cx: &mut TestAppContext) {
        let deadline = Instant::now() + DEADLINE;
        loop {
            cx.run_until_parked();
            match self.heard.recv_timeout(PAUSE) {
                Ok(Message::Shutdown) => return,
                Ok(other) => panic!("the Window sent {other:?}, and the stop frame was expected"),
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("the Window closed the connection, and the stop frame was expected")
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            assert!(Instant::now() < deadline, "the Window sent no stop frame");
        }
    }

    /// The frames of input the Window sent to `terminal` so far, in the order it sent
    /// them.
    pub fn inputs(&self, terminal: &TerminalId) -> Vec<Vec<u8>> {
        self.standing
            .lock()
            .inputs
            .iter()
            .filter(|(from, _)| from == terminal)
            .map(|(_, bytes)| bytes.clone())
            .collect()
    }

    /// Sets the reviews that every `GetReviews` of this workspace is answered with from
    /// now on. It tells the Window nothing: a test that wants it to read them tells
    /// `Fact::ReviewsChanged` after.
    pub fn set_reviews(&self, reviews: WorkspaceReviews) {
        self.reviews
            .set
            .lock()
            .expect("the reviews are not poisoned")
            .insert(reviews.workspace.clone(), reviews);
    }

    /// From now on refuses every request for reviews with this, as the Keeper of the
    /// contract does; none to answer them again.
    pub fn refuse_reviews(&self, refusal: Option<Refusal>) {
        *self.reviews.refusal.lock().expect("not poisoned") = refusal;
    }

    /// How many times the Window asked for reviews so far.
    pub fn reviews_asked(&self) -> usize {
        self.reviews.heard.load(Ordering::SeqCst)
    }

    /// From now on keeps the requests for reviews unanswered, until
    /// [`KeeperEnd::release_reviews`]: a Keeper that is slow.
    pub fn hold_reviews(&self) {
        self.reviews.holding.store(true, Ordering::SeqCst);
    }

    /// Answers the requests for reviews that were kept, in the order they came, with what
    /// is set now, and answers the ones that follow at once.
    pub fn release_reviews(&self) {
        let mut held = self.reviews.held.lock().expect("not poisoned");
        self.reviews.holding.store(false, Ordering::SeqCst);
        for (id, workspace) in held.drain(..) {
            let reply = self.reviews.answer(&workspace);
            write(&self.stream, &Message::Reply { to: id, reply });
        }
    }

    /// Answers the next request, which has to be the one for the changes of the
    /// workspace on screen, with `changes`; then the request for each of their files,
    /// which the pane makes in the order of the list, with the versions `sides` gives
    /// for its path.
    pub fn show(
        &mut self,
        changes: &Changes,
        sides: impl Fn(&ChangedFile) -> Result<FileSides, Refusal>,
        cx: &mut TestAppContext,
    ) {
        let asked = self.expect(
            Request::GetChanges {
                workspace: changes.workspace.clone(),
            },
            cx,
        );
        self.reply(asked, Ok(Reply::Changes(changes.clone())));
        for file in &changes.files {
            let asked = self.expect(read_file(changes, file), cx);
            self.reply(asked, sides(file).map(Reply::File));
        }
    }
}

/// The id of the review of no agent that every workspace has until a test says otherwise.
pub fn review_id() -> ReviewId {
    ReviewId::new("e".repeat(32))
}

/// A review of no agent with nothing in it.
pub fn no_agent_review() -> Review {
    Review {
        id: review_id(),
        agent: None,
        reviewed: None,
        attached: None,
        threads: Vec::new(),
        rounds: Vec::new(),
    }
}

/// The id of a session that is this digit thirty-two times.
pub fn session_id(digit: char) -> SessionId {
    SessionId::new(digit.to_string().repeat(32))
}

/// A session of workspace `1` that holds `review`.
pub fn session(digit: char, title: &str, state: SessionState, review: Option<ReviewId>) -> Session {
    Session {
        id: session_id(digit),
        agent: "claude".to_owned(),
        title: title.to_owned(),
        workspace: id('1'),
        terminal: None,
        state,
        turn: None,
        review,
        attached: None,
        resumable: false,
        turn_from_user: false,
    }
}

/// The id of a turn that is this digit thirty-two times.
pub fn turn_id(digit: char) -> TurnId {
    TurnId::new(digit.to_string().repeat(32))
}

/// The same session, working on `turn`, as the agents model shows a session whose turn
/// runs: a turn a message of the user started.
pub fn working_on(session: Session, turn: TurnId) -> Session {
    Session {
        state: SessionState::Works,
        turn: Some(turn),
        turn_from_user: true,
        ..session
    }
}

/// A turn of session `session` that runs, numbered `number` among its turns, counted from
/// the state `start`, with these steps so far: what `GetTurn` answers.
pub fn running_turn(
    turn: TurnId,
    session: SessionId,
    number: u32,
    start: Option<&str>,
    steps: Vec<Step>,
) -> Turn {
    Turn {
        id: turn,
        session,
        number,
        ended: None,
        start: start.map(str::to_owned),
        state: None,
        steps,
    }
}

/// The fact of step `number` of `turn` of `session` in workspace `1`, as the Keeper tells
/// it.
pub fn step_fact(session: SessionId, turn: TurnId, number: u32, step: Step) -> Fact {
    Fact::Step {
        workspace: id('1'),
        session,
        turn,
        number,
        step,
    }
}

/// The agents of workspace `1`: these sessions, in the order they started.
pub fn agents_of(sessions: Vec<Session>) -> WorkspaceAgents {
    WorkspaceAgents {
        workspace: id('1'),
        shown: sessions.first().map(|session| session.state),
        sessions,
    }
}

/// A review with nothing in it, of this session or of no agent, with this state reviewed.
pub fn review_with(review: ReviewId, agent: Option<SessionId>, reviewed: Option<String>) -> Review {
    Review {
        id: review,
        agent,
        reviewed,
        ..no_agent_review()
    }
}

/// The reviews of workspace `1`.
pub fn reviews_of(reviews: Vec<Review>) -> WorkspaceReviews {
    WorkspaceReviews {
        workspace: id('1'),
        reviews,
        viewed: Vec::new(),
    }
}

/// The request by which a Window reads the two versions of one file of `changes`.
pub fn read_file(changes: &Changes, file: &ChangedFile) -> Request {
    let (since, old_path) = match &file.status {
        FileStatus::Added => (None, file.path.clone()),
        FileStatus::Renamed { from } => (changes.counted_from.clone(), from.clone()),
        FileStatus::Modified | FileStatus::Deleted => {
            (changes.counted_from.clone(), file.path.clone())
        }
    };
    Request::ReadFile {
        workspace: changes.workspace.clone(),
        since,
        old_path,
        path: file.path.clone(),
    }
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

pub fn id(digit: char) -> WorkspaceId {
    WorkspaceId::new(digit.to_string().repeat(32))
}

pub fn workspace(digit: char, branch: &str) -> Workspace {
    Workspace {
        id: id(digit),
        path: PathBuf::from(format!("/gits/{branch}")),
        head: Some(Head::Branch(branch.to_owned())),
    }
}

pub fn repository(workspaces: Vec<Workspace>) -> Repository {
    Repository {
        id: RepositoryId::new("a".repeat(32)),
        name: "repository".to_owned(),
        workspaces,
    }
}

pub fn file(path: &str, status: FileStatus) -> ChangedFile {
    ChangedFile {
        path: PathBuf::from(path),
        status,
    }
}

pub fn commit(digit: char, summary: &str) -> Commit {
    Commit {
        id: digit.to_string().repeat(40),
        summary: summary.to_owned(),
    }
}

/// The base a repository has when nobody set one.
pub fn default_base(name: &str) -> Base {
    Base {
        name: name.to_owned(),
        source: BaseSource::Default,
    }
}

/// The changes of a branch of workspace `1` since it left `main`, counted from
/// [`COMMIT`].
pub fn since_main(commits: Vec<Commit>, files: Vec<ChangedFile>) -> Changes {
    Changes {
        workspace: id('1'),
        comparison: Comparison::SinceBase {
            base: default_base("main"),
        },
        counted_from: Some(COMMIT.to_owned()),
        commits,
        files,
    }
}

/// Two versions of a file that is text on both sides.
pub fn texts(old: &str, new: &str) -> Result<FileSides, Refusal> {
    Ok(FileSides {
        old: Content::Text(old.as_bytes().to_vec()),
        new: Content::Text(new.as_bytes().to_vec()),
    })
}

/// The view a test window holds: the pane as the Frame would lay it out, alone.
struct Host {
    pane: AnyView,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.pane.clone())
    }
}

/// A test window with the pane in it, and the Keeper the test plays.
pub struct Stage {
    /// The pane, as the model a test reads back through.
    pub pane: Entity<ReviewPane>,
    /// Where the keys go when the pane has the focus, as the pane told the Frame.
    pub focus: FocusHandle,
    /// The shared models the pane was built on.
    pub models: Models,
    /// The other end of the Window's connection.
    pub keeper: KeeperEnd,
}

/// The shared models of a window, fed by a Keeper the test plays. The Keeper has listed
/// `workspaces` of one repository, and the first of them is on screen; the request for
/// its changes is the next the Keeper hears. The keys of the pane are bound from the
/// pane's own table, the way the Window program binds them.
pub fn link_up(workspaces: Vec<Workspace>, cx: &mut TestAppContext) -> (Models, KeeperEnd) {
    link_up_staged(workspaces, |_| {}, cx)
}

/// [`link_up`], with what the Keeper knows set by `stage` before the Window is linked to
/// it, as [`KeeperEnd::attach_staged`] says: the agents a running turn is read from
/// (`set_agents`), the settings (`set_settings`), a turn (`set_turn`).
pub fn link_up_staged(
    workspaces: Vec<Workspace>,
    stage: impl FnOnce(&KeeperEnd),
    cx: &mut TestAppContext,
) -> (Models, KeeperEnd) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        bind(&(yaar_review_pane::entry().controls)(), cx);
    });
    let link = cx.new(|_| KeeperLink::new());
    let models = cx.update(|cx| Models::new(link.clone(), cx));
    let mut keeper = KeeperEnd::attach_staged(&link, cx, stage);
    let asked = keeper.expect(Request::ListWorkspaces, cx);
    keeper.reply(asked, Ok(Reply::Workspaces(vec![repository(workspaces)])));
    wait_until(cx, "a workspace on screen", |cx| {
        models
            .workspaces
            .read_with(cx, |list, _| list.selected().is_some())
    });
    (models, keeper)
}

/// Opens a window that holds the pane alone, built by the pane's own builder the way
/// the Window program builds it.
pub fn open_window(
    models: Models,
    keeper: KeeperEnd,
    cx: &mut TestAppContext,
) -> (Stage, VisualTestContext) {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: Point::default(),
            size: WINDOW,
        })),
        ..Default::default()
    };
    let mut built = None;
    let (window, _host) = cx
        .update(|cx| {
            gpui_kit::open_window(options, cx, |window, cx| {
                let pane = (yaar_review_pane::entry().build)()(&models, window, cx);
                assert_eq!(pane.name, yaar_review_pane::NAME);
                let view = pane.view.clone();
                built = Some(pane);
                cx.new(|_| Host { pane: view })
            })
        })
        .expect("open a test window");
    let built = built.expect("the pane was built");
    let pane = built
        .view
        .downcast::<ReviewPane>()
        .expect("the view of the pane is the pane");
    let cx = VisualTestContext::from_window(window, cx);
    (
        Stage {
            pane,
            focus: built.focus,
            models,
            keeper,
        },
        cx,
    )
}

/// A window with the pane in it, linked as [`link_up`] links it: the pane is there
/// before the Keeper has said anything about the changes.
pub fn open(workspaces: Vec<Workspace>, cx: &mut TestAppContext) -> (Stage, VisualTestContext) {
    open_staged(workspaces, |_| {}, cx)
}

/// [`open`], with what the Keeper knows set by `stage` before the Window is linked to it,
/// as [`link_up_staged`] says.
pub fn open_staged(
    workspaces: Vec<Workspace>,
    stage: impl FnOnce(&KeeperEnd),
    cx: &mut TestAppContext,
) -> (Stage, VisualTestContext) {
    let (models, keeper) = link_up_staged(workspaces, stage, cx);
    open_window(models, keeper, cx)
}

/// The rows of the pane, as they are now.
pub fn rows(pane: &Entity<ReviewPane>, cx: &mut TestAppContext) -> Vec<PaneRow> {
    pane.read_with(cx, |pane, _| pane.rows().to_vec())
}

/// Waits until the pane shows rows and none of its files is still on its way.
pub fn wait_for_files(pane: &Entity<ReviewPane>, cx: &mut TestAppContext) {
    wait_until(cx, "every file of the changes", |cx| {
        pane.read_with(cx, |pane, _| {
            !pane.rows().is_empty()
                && !pane.rows().iter().any(|row| {
                    matches!(
                        row,
                        PaneRow::Unshown {
                            why: Unshown::Loading,
                            ..
                        }
                    )
                })
        })
    });
}

/// Where a row of the pane was drawn in the last frame; fails when it was not laid out.
pub fn bounds_of_row(index: usize, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    drawn(&format!("review-row-{index}"), cx)
        .unwrap_or_else(|| panic!("row {index} of the pane is not laid out"))
}

/// Where the element with this selector was drawn in the last frame, if it was.
pub fn drawn(selector: &str, cx: &mut VisualTestContext) -> Option<Bounds<Pixels>> {
    // The selector has to outlive the call, and a test ends soon.
    cx.debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
}

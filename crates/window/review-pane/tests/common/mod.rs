//! What the pane's tests share: a window that holds the pane alone, and the Keeper's end of the
//! Window's connection, played on a socket pair with `yaar-link` only.

// Each test program takes what it needs of this.
#![allow(dead_code)]

pub mod navigator;

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
    SessionState, Setting, Step, Turn, TurnId, Workspace, WorkspaceAgents, WorkspaceId,
    WorkspaceReviews, read_message, write_message,
};
use yaar_review_pane::{PaneRow, ReviewPane, Unshown};

pub const DEADLINE: Duration = Duration::from_secs(10);

const PAUSE: Duration = Duration::from_millis(1);

/// Small, so that a diff of some length does not fit and the pane has to scroll.
pub const WINDOW: Size<Pixels> = size(px(900.), px(420.));

pub const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

/// What the Keeper end answers to `GetReviews` by itself, so that a request for reviews never
/// reaches the test as its next request.
#[derive(Default)]
struct Reviews {
    /// By workspace; a workspace with nothing set has one empty review of no agent.
    set: Mutex<HashMap<WorkspaceId, WorkspaceReviews>>,
    heard: AtomicUsize,
    holding: AtomicBool,
    held: Mutex<Vec<(RequestId, WorkspaceId)>>,
    refusal: Mutex<Option<Refusal>>,
}

impl Reviews {
    fn answer(&self, workspace: &WorkspaceId) -> Result<Reply, Refusal> {
        if let Some(refusal) = self.refusal.lock().expect("not poisoned").clone() {
            return Err(refusal);
        }
        Ok(Reply::Reviews(self.reviews_of(workspace)))
    }

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

/// A lock keeps the frames whole: the thread that answers the reviews and the test both write.
fn write(stream: &Mutex<UnixStream>, message: &Message) {
    let mut stream = stream.lock().expect("the stream is not poisoned");
    write_message(&mut *stream, message).expect("write a message");
}

/// Questions the Window asks at times a test does not choose; answered here from what a test set,
/// and counted, so they never reach the test as its next request.
#[derive(Default)]
struct Standing {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    agents: Vec<WorkspaceAgents>,
    clis: Vec<yaar_link::AgentCli>,
    /// By workspace; with none set, a history with no entry that starts where the last answered
    /// changes are counted from.
    histories: HashMap<WorkspaceId, History>,
    counted_from: HashMap<WorkspaceId, Option<String>>,
    uncommitted: Vec<CreditedFile>,
    /// Each `SetSetting` takes the place of the setting it is the same thing as, or comes last.
    settings: Vec<Setting>,
    settings_set: Vec<Setting>,
    /// A turn with none set is refused as unknown.
    turns: HashMap<TurnId, Turn>,
    turns_asked: Vec<TurnId>,
    uncommitted_review: Option<ReviewId>,
    asked: Asked,
    agents_held: Held,
}

#[derive(Default, Clone, Copy)]
struct Asked {
    history: usize,
    uncommitted: usize,
}

/// Answers a test keeps back, as a slow Keeper would: made when heard, sent when released.
#[derive(Default)]
struct Held {
    holding: bool,
    held: VecDeque<(RequestId, Reply)>,
}

enum Heard {
    Elsewhere,
    Answer(Result<Reply, Refusal>),
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
                let reply = Reply::Agents(inner.agents.clone());
                inner.agents_held.offer(id, reply)
            }
            Request::ListClis => Heard::Answer(Ok(Reply::Clis(inner.clis.clone()))),
            Request::ListTerminals => Heard::Answer(Ok(Reply::Terminals(Vec::new()))),
            Request::ShowWorkspace { .. } => Heard::Answer(Ok(Reply::Done)),
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
            Request::GetSettings => Heard::Answer(Ok(Reply::Settings(inner.settings.clone()))),
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

pub struct KeeperEnd {
    stream: Arc<Mutex<UnixStream>>,
    // Read by a thread of this end's own, so the test thread can keep the app running while it
    // waits.
    heard: Receiver<Message>,
    reviews: Arc<Reviews>,
    standing: Arc<Standing>,
}

impl KeeperEnd {
    /// Runs `stage` before the link is made: the Window asks for the agents and the settings as
    /// soon as it is linked, so they have to be set before.
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
                            // A Window that is gone no longer reads: the next read ends this
                            // thread.
                            Heard::Answer(reply) => {
                                let mut stream =
                                    stream.lock().unwrap_or_else(PoisonError::into_inner);
                                let _ =
                                    write_message(&mut *stream, &Message::Reply { to: id, reply });
                            }
                            Heard::Kept => {}
                        },
                        message => {
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

    pub fn expect(&mut self, wanted: Request, cx: &mut TestAppContext) -> RequestId {
        let (id, request) = self.request(cx);
        assert_eq!(request, wanted);
        id
    }

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

    pub fn tell(&mut self, fact: Fact) {
        write(&self.stream, &Message::Fact(fact));
    }

    /// Tells the Window nothing: a test that wants it to read them tells `Fact::AgentsChanged`.
    pub fn set_agents(&self, agents: Vec<WorkspaceAgents>) {
        self.standing.lock().agents = agents;
    }

    /// Set before the Window connects: it asks then.
    pub fn set_clis(&self, clis: Vec<yaar_link::AgentCli>) {
        self.standing.lock().clis = clis;
    }

    pub fn tell_agents(&mut self, agents: WorkspaceAgents) {
        self.set_agents(vec![agents.clone()]);
        self.tell(Fact::AgentsChanged(agents));
    }

    pub fn set_history(&self, history: History) {
        let mut inner = self.standing.lock();
        inner.histories.insert(history.workspace.clone(), history);
    }

    pub fn set_uncommitted(&self, uncommitted: Vec<CreditedFile>) {
        self.standing.lock().uncommitted = uncommitted;
    }

    pub fn uncommitted_review(&self) -> Option<ReviewId> {
        self.standing.lock().uncommitted_review.clone()
    }

    /// Tells the Window nothing: a test that wants it to hear one tells `Fact::SettingChanged`.
    pub fn set_settings(&self, settings: Vec<Setting>) {
        self.standing.lock().settings = settings;
    }

    pub fn settings_set(&self) -> Vec<Setting> {
        self.standing.lock().settings_set.clone()
    }

    pub fn set_turn(&self, turn: Turn) {
        self.standing.lock().turns.insert(turn.id.clone(), turn);
    }

    pub fn turns_asked(&self) -> Vec<TurnId> {
        self.standing.lock().turns_asked.clone()
    }

    pub fn history_asked(&self) -> usize {
        self.standing.lock().asked.history
    }

    pub fn uncommitted_asked(&self) -> usize {
        self.standing.lock().asked.uncommitted
    }

    pub fn hold_agents(&self) {
        self.standing.lock().agents_held.holding = true;
    }

    pub fn held_agents(&self) -> usize {
        self.standing.lock().agents_held.held.len()
    }

    pub fn release_agents(&self, count: usize) {
        let released = self.standing.lock().agents_held.release(count);
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

    /// Tells the Window nothing: a test that wants it to read them tells `Fact::ReviewsChanged`.
    pub fn set_reviews(&self, reviews: WorkspaceReviews) {
        self.reviews
            .set
            .lock()
            .expect("the reviews are not poisoned")
            .insert(reviews.workspace.clone(), reviews);
    }

    pub fn refuse_reviews(&self, refusal: Option<Refusal>) {
        *self.reviews.refusal.lock().expect("not poisoned") = refusal;
    }

    pub fn reviews_asked(&self) -> usize {
        self.reviews.heard.load(Ordering::SeqCst)
    }

    pub fn hold_reviews(&self) {
        self.reviews.holding.store(true, Ordering::SeqCst);
    }

    /// Answers the kept requests in order with what is set now, and the ones that follow at once.
    pub fn release_reviews(&self) {
        let mut held = self.reviews.held.lock().expect("not poisoned");
        self.reviews.holding.store(false, Ordering::SeqCst);
        for (id, workspace) in held.drain(..) {
            let reply = self.reviews.answer(&workspace);
            write(&self.stream, &Message::Reply { to: id, reply });
        }
    }

    /// Answers the request for the changes of the workspace on screen, then for each file in list
    /// order, with the versions `sides` gives.
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

pub fn session_id(digit: char) -> SessionId {
    SessionId::new(digit.to_string().repeat(32))
}

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

pub fn turn_id(digit: char) -> TurnId {
    TurnId::new(digit.to_string().repeat(32))
}

pub fn working_on(session: Session, turn: TurnId) -> Session {
    Session {
        state: SessionState::Works,
        turn: Some(turn),
        turn_from_user: true,
        ..session
    }
}

/// The same session, working on a `turn` the CLI started itself: a task it finished, a hook it ran.
pub fn self_started(session: Session, turn: TurnId) -> Session {
    Session {
        turn_from_user: false,
        ..working_on(session, turn)
    }
}

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

pub fn step_fact(session: SessionId, turn: TurnId, number: u32, step: Step) -> Fact {
    Fact::Step {
        workspace: id('1'),
        session,
        turn,
        number,
        step,
    }
}

pub fn agents_of(sessions: Vec<Session>) -> WorkspaceAgents {
    WorkspaceAgents {
        workspace: id('1'),
        shown: sessions.first().map(|session| session.state),
        sessions,
    }
}

pub fn review_with(review: ReviewId, agent: Option<SessionId>, reviewed: Option<String>) -> Review {
    Review {
        id: review,
        agent,
        reviewed,
        ..no_agent_review()
    }
}

pub fn reviews_of(reviews: Vec<Review>) -> WorkspaceReviews {
    WorkspaceReviews {
        workspace: id('1'),
        reviews,
        viewed: Vec::new(),
    }
}

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

pub fn default_base(name: &str) -> Base {
    Base {
        name: name.to_owned(),
        source: BaseSource::Default,
    }
}

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

pub fn texts(old: &str, new: &str) -> Result<FileSides, Refusal> {
    Ok(FileSides {
        old: Content::Text(old.as_bytes().to_vec()),
        new: Content::Text(new.as_bytes().to_vec()),
    })
}

struct Host {
    pane: AnyView,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.pane.clone())
    }
}

pub struct Stage {
    pub pane: Entity<ReviewPane>,
    pub focus: FocusHandle,
    pub models: Models,
    pub keeper: KeeperEnd,
}

/// The Keeper has listed `workspaces` of one repository, the first on screen; the request for its
/// changes is the next the Keeper hears.
pub fn link_up(workspaces: Vec<Workspace>, cx: &mut TestAppContext) -> (Models, KeeperEnd) {
    link_up_staged(workspaces, |_| {}, cx)
}

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

pub fn open(workspaces: Vec<Workspace>, cx: &mut TestAppContext) -> (Stage, VisualTestContext) {
    open_staged(workspaces, |_| {}, cx)
}

pub fn open_staged(
    workspaces: Vec<Workspace>,
    stage: impl FnOnce(&KeeperEnd),
    cx: &mut TestAppContext,
) -> (Stage, VisualTestContext) {
    let (models, keeper) = link_up_staged(workspaces, stage, cx);
    open_window(models, keeper, cx)
}

pub fn rows(pane: &Entity<ReviewPane>, cx: &mut TestAppContext) -> Vec<PaneRow> {
    pane.read_with(cx, |pane, _| pane.rows().to_vec())
}

pub fn wait_for_files(pane: &Entity<ReviewPane>, cx: &mut TestAppContext) {
    wait_until(cx, "every file of the changes", |cx| {
        pane.read_with(cx, |pane, _| {
            (!pane.rows().is_empty() || pane.comparison().is_some())
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

pub fn bounds_of_row(index: usize, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    drawn(&format!("review-row-{index}"), cx)
        .unwrap_or_else(|| panic!("row {index} of the pane is not laid out"))
}

pub fn drawn(selector: &str, cx: &mut VisualTestContext) -> Option<Bounds<Pixels>> {
    // The selector has to outlive the call, and a test ends soon.
    cx.debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
}

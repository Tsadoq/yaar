// Each test program takes what it needs of this.
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::HashMap;
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
    Head, Hello, Message, PROTOCOL_VERSION, Refusal, Reply, Repository, RepositoryId, Request,
    RequestId, Setting, TerminalId, TerminalInfo, TerminalSize, Workspace, WorkspaceAgents,
    WorkspaceId, read_message, write_message,
};
use yaar_terminal_pane::TerminalPane;

pub const DEADLINE: Duration = Duration::from_secs(10);

const PAUSE: Duration = Duration::from_millis(1);

pub const WINDOW: Size<Pixels> = size(px(1200.), px(700.));

/// Prepares the app as `yaar_window::init` does, minus the review pane. Without
/// `release_program_keys`, Tab, Shift-Tab and, off macOS, Ctrl-C and Ctrl-Shift-I would be
/// lost in the app instead of reaching the terminal.
pub fn init_as_the_app_does(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        yaar_frame::release_program_keys(cx);
        let mut table = controls();
        table.extend(yaar_terminal_pane::controls());
        bind(&table, cx);
    });
}

pub fn terminal(character: char) -> TerminalId {
    TerminalId::new(character.to_string().repeat(32))
}

pub fn workspace_id(digit: char) -> WorkspaceId {
    WorkspaceId::new(digit.to_string().repeat(32))
}

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

pub fn two_workspaces() -> Repository {
    let mut repository = one_repository();
    repository.workspaces.push(Workspace {
        id: workspace_id('2'),
        path: PathBuf::from("/gits/other"),
        head: Some(Head::Branch("other".to_owned())),
    });
    repository
}

type Built = (Entity<TerminalPane>, FocusHandle);

/// Questions the Keeper answers itself, so they never reach the test as its next request.
#[derive(Default)]
struct Standing {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    agents: Vec<WorkspaceAgents>,
    clis: Vec<yaar_link::AgentCli>,
    terminals: Vec<TerminalInfo>,
    settings: Vec<Setting>,
    inputs: Vec<(TerminalId, Vec<u8>)>,
}

enum Heard {
    Elsewhere,
    Answer(Result<Reply, Refusal>),
}

impl Standing {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn hear(&self, request: &Request) -> Heard {
        let mut inner = self.lock();
        match request {
            Request::ListAgents => Heard::Answer(Ok(Reply::Agents(inner.agents.clone()))),
            // Asked once for each connection, at a moment the test does not choose.
            Request::ListClis => Heard::Answer(Ok(Reply::Clis(inner.clis.clone()))),
            Request::ListTerminals => Heard::Answer(Ok(Reply::Terminals(inner.terminals.clone()))),
            Request::ShowWorkspace { .. } => Heard::Answer(Ok(Reply::Done)),
            Request::GetSettings => Heard::Answer(Ok(Reply::Settings(inner.settings.clone()))),
            Request::SetSetting { setting } => {
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

/// The Keeper, played by the test. A thread of its own reads what the Window sends and answers
/// resizes, the changes question and the `Standing` questions itself; other requests wait for
/// `request`, input for `input`.
pub struct TestKeeper {
    writer: Arc<Mutex<UnixStream>>,
    requests: Receiver<(RequestId, Request)>,
    inputs: Receiver<(TerminalId, Vec<u8>)>,
    sizes: Arc<Mutex<Vec<(TerminalId, TerminalSize)>>>,
    standing: Arc<Standing>,
}

impl TestKeeper {
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
                        // No pane of these tests shows the workspace's changes: this end says it
                        // keeps none.
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
                        Message::Request { id, request } => match standing.hear(&request) {
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
                        },
                        Message::Input { terminal, bytes } => {
                            tap(&terminal, &bytes);
                            standing
                                .lock()
                                .inputs
                                .push((terminal.clone(), bytes.clone()));
                            if tell_input.send((terminal, bytes)).is_err() {
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
            sizes,
            standing,
        };
        // Before the Window is linked: it asks for the agents and the terminals the moment it is,
        // so what the first answers hold has to be set first.
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

    pub fn input(&self, cx: &mut TestAppContext) -> (TerminalId, Vec<u8>) {
        self.next(&self.inputs, "input", cx)
    }

    /// The input of `terminal` before `ending`, read up to and including it; fails when another
    /// terminal's input comes first. To know a key sent nothing, type known text after it.
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

    pub fn no_input_waits(&self) -> bool {
        self.inputs.try_recv().is_err()
    }

    pub fn resizes(&self, terminal: &TerminalId) -> Vec<TerminalSize> {
        self.sizes
            .lock()
            .expect("the sizes")
            .iter()
            .filter(|(id, _)| id == terminal)
            .map(|(_, size)| *size)
            .collect()
    }

    /// The Window asks when it connects, so a test sets them before.
    pub fn set_clis(&self, clis: Vec<yaar_link::AgentCli>) {
        self.standing.lock().clis = clis;
    }

    pub fn set_terminals(&self, terminals: Vec<TerminalInfo>) {
        self.standing.lock().terminals = terminals;
    }

    pub fn inputs(&self, terminal: &TerminalId) -> Vec<Vec<u8>> {
        self.standing
            .lock()
            .inputs
            .iter()
            .filter(|(from, _)| from == terminal)
            .map(|(_, bytes)| bytes.clone())
            .collect()
    }

    pub fn reply(&self, to: RequestId, reply: Result<Reply, Refusal>) {
        self.write(&Message::Reply { to, reply });
    }

    pub fn list(&self, listed: &[Repository], cx: &mut TestAppContext) {
        let (id, request) = self.request(cx);
        assert_eq!(request, Request::ListWorkspaces);
        self.reply(id, Ok(Reply::Workspaces(listed.to_vec())));
    }

    pub fn open(
        &mut self,
        terminal: &TerminalId,
        cx: &mut TestAppContext,
    ) -> (WorkspaceId, TerminalSize) {
        self.open_running(terminal, yaar_link::Program::Shell, cx)
    }

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

    pub fn output(&self, terminal: &TerminalId, bytes: &[u8]) {
        self.write(&Message::Output {
            terminal: terminal.clone(),
            bytes: bytes.to_vec(),
        });
    }

    /// What an `AgentsChanged` fact says of a workspace's agents answers every later `ListAgents`:
    /// the Window asks again when a change crosses an answer, and an older answer would drop the
    /// fact's sessions.
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

pub struct Panes {
    pub pane: Entity<TerminalPane>,
    pub pane_focus: FocusHandle,
    pub models: Models,
    pub keeper: TestKeeper,
    opened: HashMap<TerminalId, TerminalSize>,
}

pub fn open_panes_tapping(
    cx: &mut TestAppContext,
    tap: impl FnMut(&TerminalId, &[u8]) + Send + 'static,
) -> (Panes, VisualTestContext) {
    open_panes_listing(cx, &[one_repository()], tap)
}

pub fn open_panes_listing(
    cx: &mut TestAppContext,
    listed: &[Repository],
    tap: impl FnMut(&TerminalId, &[u8]) + Send + 'static,
) -> (Panes, VisualTestContext) {
    open_panes_staged(cx, listed, |_| {}, tap)
}

pub fn open_panes_staged(
    cx: &mut TestAppContext,
    listed: &[Repository],
    stage: impl FnOnce(&TestKeeper),
    tap: impl FnMut(&TerminalId, &[u8]) + Send + 'static,
) -> (Panes, VisualTestContext) {
    init_as_the_app_does(cx);
    let link = cx.new(|_| KeeperLink::new());
    let models = cx.update(|cx| Models::new(link.clone(), cx));

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
    let (window, _) = cx
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
    // GPUI tells no focus change to a window that is not active.
    cx.update(|window, _| window.activate_window());
    // The first thing the Keeper hears must be the question for the list.
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
            models,
            keeper,
            opened: HashMap::new(),
        },
        cx,
    )
}

pub fn open_panes(cx: &mut TestAppContext) -> (Panes, VisualTestContext) {
    open_panes_tapping(cx, |_, _| {})
}

impl Panes {
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

    pub fn open_terminal(&mut self, id: &TerminalId, cx: &mut VisualTestContext) {
        self.press_in_the_app::<yaar_terminal_pane::NewTerminal>(cx);
        self.answer_open(id, yaar_link::Program::Shell, cx);
    }

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

    /// Clicks in `elsewhere` before the key: on Linux a key of the app does nothing while a
    /// terminal has the focus.
    pub fn press_in_the_app<A: gpui_kit::Action + Default>(&mut self, cx: &mut VisualTestContext) {
        let key = key_of::<A>();
        self.click_elsewhere(cx);
        cx.simulate_keystrokes(key);
    }

    pub fn grid(&self, terminal: &TerminalId) -> TerminalSize {
        self.keeper
            .resizes(terminal)
            .last()
            .copied()
            .or_else(|| self.opened.get(terminal).copied())
            .unwrap_or_else(|| panic!("terminal {terminal} was not opened by this test"))
    }

    /// The middle of a cell, estimated as the slot's size over the grid; the estimate stays inside
    /// the right cell only for the first five columns and lines, where tests aim.
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

    pub fn click_elsewhere(&mut self, cx: &mut VisualTestContext) {
        click_in("elsewhere-pane", cx);
    }

    pub fn click_slot(&mut self, slot: usize, cx: &mut VisualTestContext) {
        click_in(&format!("terminal-slot-{slot}"), cx);
    }
}

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

pub fn drawn(selector: &str, cx: &mut VisualTestContext) -> Option<Bounds<Pixels>> {
    // The test context takes a text that lives as long as the program.
    cx.debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
}

pub fn click_in(selector: &str, cx: &mut VisualTestContext) {
    let at = drawn(selector, cx)
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
        .center();
    cx.simulate_click(at, Modifiers::none());
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

pub fn lines(pane: &Entity<TerminalPane>, id: &TerminalId, cx: &impl AppContext) -> Vec<String> {
    let view = pane
        .read_with(cx, |pane, _| pane.terminal(id))
        .expect("a terminal of the pane");
    view.read_with(cx, |view, _| view.lines())
}

pub fn refused(reason: &str) -> Refusal {
    Refusal::Failed {
        reason: reason.to_owned(),
    }
}

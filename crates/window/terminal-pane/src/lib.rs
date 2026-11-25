//! Terminal pane: the terminals of the workspace, their launchers, and the attachment shown at an
//! agent's terminal. The Keeper owns the terminals and their programs: closing the window ends
//! none.

mod attachment;
mod consent;
mod draw;
mod emulator;
mod keys;
mod launchers;
mod modes;
mod mouse;
mod view;

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::*;
use yaar_frame::{AskError, Asks, BuildPane, Control, Models, Pane, PaneEntry, Place, Sends};
use yaar_link::{
    Ending, Fact, Program, Reply, Request, RequestKind, SessionState, TerminalId, TerminalInfo,
    TerminalPlace, TerminalSize, WorkspaceId,
};
use yaar_ui::metrics::{SPACE_2, SPACE_3, SPACE_4, SPACE_5};
use yaar_ui::pieces::{Banner, Chip, Divider, EmptyState, SectionLabel, Tone};
use yaar_ui::themes::palette;

pub use attachment::ADDRESS_EACH_COMMENT;
pub use consent::{NO_LABEL, NOTHING_TO_INSTALL, YES_LABEL};
pub use draw::colour_of;
pub use emulator::{Cell, CellPlace, Color, Cursor, Emulator, Progress, Screen, Style};
pub use keys::{OptionKey, encode_focus, encode_key, encode_paste, encode_release};
pub use modes::{KeyboardFlags, Modes, MouseMode};
pub use mouse::{MouseAction, MouseButton, encode_mouse};
pub use view::{TerminalView, ViewEvent};

actions!(
    terminal,
    [
        NewTerminal,
        NextTerminal,
        PreviousTerminal,
        ToggleSplit,
        Copy,
        Paste,
        ScrollPageUp,
        ScrollPageDown,
        ViewAttachment,
        DetachAttachment,
        AddressEachComment,
        NewAgentTerminal,
        NextAgentCli,
        ResumeAgent,
        InstallAgentExtensions,
        /// Answers no: nothing is installed, and an installed entry is taken out.
        KeepAgentExtensionsOut,
        Clear,
        SelectAll,
        ScrollToTop,
        ScrollToBottom,
        ScrollLineUp,
        ScrollLineDown,
    ]
);

pub const NAME: &str = "terminal";

pub const KEY_CONTEXT: &str = "TerminalPane";

pub const FIRST_SIZE: TerminalSize = TerminalSize {
    columns: 80,
    rows: 24,
};

pub const WORKTREE_BEING_MADE: &str = "Making the worktree of";

/// How long after a key the pane asks for a frame at every frame: a program's answer is drawn in
/// the first frame after it arrives.
pub const FRAMES_AFTER_A_KEY: Duration = Duration::from_secs(1);

pub fn entry() -> PaneEntry {
    PaneEntry {
        name: NAME,
        controls,
        build: pane,
    }
}

pub fn pane() -> BuildPane {
    Box::new(|models, window, cx| {
        let view = cx.new(|cx| TerminalPane::new(models, window, cx));
        let focus = view.read(cx).focus.clone();
        Pane {
            name: NAME,
            view: view.into(),
            focus,
        }
    })
}

/// Kept keys work inside a terminal only, on a key no program is given; the others work anywhere.
pub fn controls() -> Vec<Control> {
    let in_a_terminal = Place::In(yaar_frame::PROGRAM_CONTEXT);
    let nothing = |title, action: fn() -> Box<dyn Action>, keystroke, place| Control {
        title,
        action,
        keystroke,
        place,
        sends: None,
        asks: Asks::Nothing,
    };
    let mut table = vec![
        Control {
            title: "New Terminal",
            action: || Box::new(NewTerminal),
            keystroke: "secondary-t",
            place: Place::Anywhere,
            sends: Some(Sends::Request(RequestKind::OpenTerminal)),
            asks: Asks::Nothing,
        },
        nothing(
            "Next Terminal",
            || Box::new(NextTerminal),
            "secondary-]",
            Place::Anywhere,
        ),
        nothing(
            "Previous Terminal",
            || Box::new(PreviousTerminal),
            "secondary-[",
            Place::Anywhere,
        ),
        nothing(
            "Split View",
            || Box::new(ToggleSplit),
            "secondary-d",
            Place::Anywhere,
        ),
        nothing(
            "Copy",
            || Box::new(Copy),
            if cfg!(target_os = "macos") {
                "cmd-c"
            } else {
                "ctrl-shift-c"
            },
            in_a_terminal,
        ),
        nothing(
            "Paste",
            || Box::new(Paste),
            if cfg!(target_os = "macos") {
                "cmd-v"
            } else {
                "ctrl-shift-v"
            },
            in_a_terminal,
        ),
        nothing(
            "Scroll Up",
            || Box::new(ScrollPageUp),
            "shift-pageup",
            in_a_terminal,
        ),
        nothing(
            "Scroll Down",
            || Box::new(ScrollPageDown),
            "shift-pagedown",
            in_a_terminal,
        ),
        nothing(
            "View Attachment",
            || Box::new(ViewAttachment),
            "secondary-alt-v",
            Place::Anywhere,
        ),
        Control {
            title: "Detach Review",
            action: || Box::new(DetachAttachment),
            keystroke: "secondary-alt-x",
            place: Place::Anywhere,
            sends: Some(Sends::Request(RequestKind::DetachReview)),
            asks: Asks::Nothing,
        },
        nothing(
            "Address Each Comment",
            || Box::new(AddressEachComment),
            "secondary-alt-a",
            Place::Anywhere,
        ),
        Control {
            title: "New Agent Terminal",
            action: || Box::new(NewAgentTerminal),
            keystroke: "secondary-shift-t",
            place: Place::Anywhere,
            sends: Some(Sends::Request(RequestKind::OpenTerminal)),
            asks: Asks::Nothing,
        },
        nothing(
            "Next Agent CLI",
            || Box::new(NextAgentCli),
            // Not Ctrl-Alt-T, which the Linux desktops give to opening a terminal of their own
            // before the app sees it.
            "secondary-alt-c",
            Place::Anywhere,
        ),
        Control {
            title: "Resume Agent",
            action: || Box::new(ResumeAgent),
            keystroke: "secondary-shift-r",
            place: Place::Anywhere,
            sends: Some(Sends::Request(RequestKind::OpenTerminal)),
            asks: Asks::Nothing,
        },
        // Not Cmd-Alt-I, the toolkit's inspector on macOS.
        Control {
            title: "Install Agent Extensions",
            action: || Box::new(InstallAgentExtensions),
            keystroke: "secondary-alt-e",
            place: Place::Anywhere,
            sends: Some(Sends::Request(RequestKind::AnswerConsent)),
            asks: Asks::Nothing,
        },
        Control {
            title: "Keep Agent Extensions Out",
            action: || Box::new(KeepAgentExtensionsOut),
            keystroke: "secondary-alt-shift-e",
            place: Place::Anywhere,
            sends: Some(Sends::Request(RequestKind::AnswerConsent)),
            asks: Asks::Nothing,
        },
    ];
    table.extend(mac_controls());
    table
}

#[cfg(target_os = "macos")]
fn mac_controls() -> Vec<Control> {
    let in_a_terminal = Place::In(yaar_frame::PROGRAM_CONTEXT);
    let nothing = |title, action: fn() -> Box<dyn Action>, keystroke| Control {
        title,
        action,
        keystroke,
        place: in_a_terminal,
        sends: None,
        asks: Asks::Nothing,
    };
    vec![
        nothing("Clear", || Box::new(Clear), "cmd-k"),
        nothing("Select All", || Box::new(SelectAll), "cmd-a"),
        nothing("Scroll to Top", || Box::new(ScrollToTop), "cmd-home"),
        nothing("Scroll to Bottom", || Box::new(ScrollToBottom), "cmd-end"),
        nothing("Scroll Line Up", || Box::new(ScrollLineUp), "cmd-up"),
        nothing("Scroll Line Down", || Box::new(ScrollLineDown), "cmd-down"),
    ]
}

#[cfg(not(target_os = "macos"))]
fn mac_controls() -> Vec<Control> {
    Vec::new()
}

struct Entry {
    id: TerminalId,
    workspace: WorkspaceId,
    view: Entity<TerminalView>,
    cut: bool,
    _events: Subscription,
}

struct Attaching {
    size: TerminalSize,
    ended: Option<Ending>,
}

#[derive(Default)]
struct Layout {
    shown: Vec<TerminalId>,
    focus_slot: usize,
}

impl Layout {
    fn put(&mut self, id: TerminalId) {
        match self.shown.get_mut(self.focus_slot) {
            Some(slot) => *slot = id,
            None => {
                self.shown.push(id);
                self.focus_slot = self.shown.len() - 1;
            }
        }
    }

    fn offer(&mut self, id: TerminalId) -> bool {
        let empty = self.shown.is_empty();
        if empty {
            self.put(id);
        }
        empty
    }

    fn focused(&self) -> Option<&TerminalId> {
        self.shown.get(self.focus_slot)
    }
}

pub struct TerminalPane {
    models: Models,
    focus: FocusHandle,
    window: AnyWindowHandle,
    size: TerminalSize,
    /// What a slot holds: the size a view last fitted while its workspace was split. A view that
    /// fits its starting size reports nothing, so this may stay unknown; reset when the room
    /// changes.
    slot: Option<TerminalSize>,
    room: Option<TerminalSize>,
    room_pixels: Option<(Size<Pixels>, Size<Pixels>)>,
    room_seen: Rc<std::cell::Cell<Option<TerminalSize>>>,
    header_height: Rc<std::cell::Cell<Option<Pixels>>>,
    notice: Option<String>,
    terminals: Vec<Entry>,
    on_screen: Option<WorkspaceId>,
    layouts: HashMap<WorkspaceId, Layout>,
    frames_until: Option<Instant>,
    focus_on_arrival: Option<WorkspaceId>,
    window_title: Option<String>,
    attaching: HashMap<TerminalId, Attaching>,
    attachments: attachment::Attachments,
    launchers: launchers::Launchers,
    _subscriptions: Vec<Subscription>,
}

impl TerminalPane {
    pub fn new(models: &Models, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let pane = cx.entity();
        models.routes.route(&pane, Self::new_terminal);
        models.routes.route(&pane, Self::next_terminal);
        models.routes.route(&pane, Self::previous_terminal);
        models.routes.route(&pane, Self::toggle_split);
        models.routes.route(&pane, Self::view_attachment);
        models.routes.route(&pane, Self::detach_attachment);
        models.routes.route(&pane, Self::address_each_comment);
        models.routes.route(&pane, Self::new_agent_terminal);
        models.routes.route(&pane, Self::next_agent_cli);
        models.routes.route(&pane, Self::resume_agent);
        models.routes.route(&pane, Self::install_agent_extensions);
        models.routes.route(&pane, Self::keep_agent_extensions_out);

        let focus = cx.focus_handle();
        let handed_on = cx.on_focus(&focus, window, |pane, window, cx| {
            pane.hand_focus_on(window, cx);
        });
        // A key that runs an action never reaches a key listener, so keys are counted here, before
        // any binding.
        let weak = cx.weak_entity();
        let keys = cx.intercept_keystrokes(move |_, window, cx| {
            weak.update(cx, |pane, cx| pane.key_pressed(window, cx))
                .ok();
        });
        let facts = cx.subscribe(&models.link, |pane, _, fact: &Fact, cx| {
            pane.heard(fact, cx);
        });
        let workspaces = cx.observe_in(&models.workspaces, window, |pane, _, window, cx| {
            pane.workspace_changed(window, cx);
        });
        let agents = cx.observe(&models.agents, |_, _, cx| cx.notify());
        let consents = cx.observe(&models.consents, |_, _, cx| cx.notify());
        let settings = cx.observe(&models.settings, |pane, _, cx| {
            pane.slot = None;
            cx.notify();
        });
        let terminals = cx.observe(&models.terminals, |pane, _, cx| pane.attach_missing(cx));
        let wanted = cx.subscribe_in(
            &models.terminals,
            window,
            |pane, _, wanted: &yaar_frame::Wanted, window, cx| {
                pane.agent_wanted(wanted, window, cx);
            },
        );
        let on_screen = models.workspaces.read(cx).selected().cloned();
        let mut pane = Self {
            models: models.clone(),
            focus,
            window: window.window_handle(),
            size: FIRST_SIZE,
            slot: None,
            room: None,
            room_pixels: None,
            room_seen: Rc::default(),
            header_height: Rc::default(),
            notice: None,
            terminals: Vec::new(),
            on_screen,
            layouts: HashMap::new(),
            frames_until: None,
            focus_on_arrival: None,
            window_title: None,
            attaching: HashMap::new(),
            attachments: attachment::Attachments::default(),
            launchers: launchers::Launchers::default(),
            _subscriptions: vec![
                handed_on, keys, facts, workspaces, agents, consents, settings, terminals, wanted,
            ],
        };
        pane.attach_missing(cx);
        pane
    }

    pub fn terminals(&self) -> Vec<TerminalId> {
        self.terminals
            .iter()
            .map(|entry| entry.id.clone())
            .collect()
    }

    pub fn terminal(&self, id: &TerminalId) -> Option<Entity<TerminalView>> {
        self.entry(id).map(|entry| entry.view.clone())
    }

    pub fn workspace(&self, id: &TerminalId) -> Option<WorkspaceId> {
        self.entry(id).map(|entry| entry.workspace.clone())
    }

    pub fn shown(&self) -> Vec<TerminalId> {
        self.layout()
            .map(|layout| layout.shown.clone())
            .unwrap_or_default()
    }

    pub fn split(&self) -> bool {
        self.layout().is_some_and(|layout| layout.shown.len() == 2)
    }

    pub fn focused(&self) -> Option<TerminalId> {
        self.layout()?.focused().cloned()
    }

    pub fn notice(&self) -> Option<String> {
        self.notice.clone()
    }

    pub fn waiting(&self, terminal: &TerminalId, cx: &App) -> bool {
        self.models
            .agents
            .read(cx)
            .in_terminal(terminal)
            .is_some_and(|session| session.state == SessionState::WaitsForPermission)
    }

    pub fn attached_cut(&self, terminal: &TerminalId) -> bool {
        self.entry(terminal).is_some_and(|entry| entry.cut)
    }

    fn entry(&self, id: &TerminalId) -> Option<&Entry> {
        self.terminals.iter().find(|entry| entry.id == *id)
    }

    fn layout(&self) -> Option<&Layout> {
        self.layouts.get(self.on_screen.as_ref()?)
    }

    fn layout_mut(&mut self) -> Option<&mut Layout> {
        Some(self.layouts.entry(self.on_screen.clone()?).or_default())
    }

    fn terminals_here(&self) -> Vec<TerminalId> {
        self.terminals
            .iter()
            .filter(|entry| Some(&entry.workspace) == self.on_screen.as_ref())
            .map(|entry| entry.id.clone())
            .collect()
    }

    fn workspace_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = self.models.workspaces.read(cx).selected().cloned();
        if now == self.on_screen {
            return;
        }
        // Read before the frame that leaves the old terminal is drawn.
        let had_the_focus = self.focus.contains_focused(window, cx);
        self.on_screen = now;
        let arrived = self.focus_on_arrival.is_some() && self.focus_on_arrival == self.on_screen;
        if arrived {
            self.focus_on_arrival = None;
        }
        if had_the_focus || arrived {
            if self.focused().is_some() {
                self.focus_terminal(window, cx);
            } else {
                self.focus.focus(window, cx);
            }
        }
        cx.notify();
    }

    fn new_terminal(&mut self, _: &NewTerminal, _: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace) = self.models.workspaces.read(cx).selected().cloned() else {
            return;
        };
        self.notice = None;
        let request = Request::OpenTerminal {
            workspace: workspace.clone(),
            program: Program::Shell,
            size: self.size,
        };
        let asked = self
            .models
            .link
            .update(cx, |link, cx| link.ask(request, cx));
        cx.spawn(async move |pane, cx| {
            let answer = asked.await;
            pane.update(cx, |pane, cx| {
                pane.opened(workspace, Program::Shell, answer, cx)
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn opened(
        &mut self,
        workspace: WorkspaceId,
        program: Program,
        answer: Result<Reply, AskError>,
        cx: &mut Context<Self>,
    ) {
        match answer {
            Ok(Reply::TerminalOpened { terminal }) => {
                self.add_opened(workspace, program, terminal, cx);
            }
            Ok(_) => self.notice = Some(AskError::Unexpected.to_string()),
            Err(error) => self.notice = Some(error.to_string()),
        }
        cx.notify();
    }

    /// The view takes the terminal's output at once, so that nothing the program wrote since
    /// the reply is lost. The terminal is not yet shown.
    fn make_entry(
        &mut self,
        workspace: WorkspaceId,
        id: TerminalId,
        size: TerminalSize,
        cut: bool,
        cx: &mut Context<Self>,
    ) -> Entity<TerminalView> {
        let link = self.models.link.clone();
        let settings = self.models.settings.clone();
        let output = link.update(cx, |link, _| link.take_output(&id));
        let view =
            cx.new(|cx| TerminalView::new(id.clone(), size, link, settings, output, cut, cx));
        let of = workspace.clone();
        let events = cx.subscribe(&view, move |pane, _, event: &ViewEvent, _| {
            pane.viewed(&of, event);
        });
        self.terminals.push(Entry {
            id,
            workspace,
            view: view.clone(),
            cut,
            _events: events,
        });
        view
    }

    fn add_opened(
        &mut self,
        workspace: WorkspaceId,
        program: Program,
        id: TerminalId,
        cx: &mut Context<Self>,
    ) {
        if self.entry(&id).is_some() {
            return;
        }
        let size = self.size;
        let view = self.make_entry(workspace.clone(), id.clone(), size, false, cx);
        self.layouts
            .entry(workspace.clone())
            .or_default()
            .put(id.clone());
        let info = TerminalInfo {
            id,
            workspace: workspace.clone(),
            program,
        };
        self.models
            .terminals
            .update(cx, |list, cx| list.opened(info, cx));
        if self.on_screen.as_ref() != Some(&workspace) {
            return;
        }
        let window = self.window;
        cx.defer(move |cx| {
            window
                .update(cx, |_, window, cx| {
                    view.read(cx).focus_handle().focus(window, cx)
                })
                .ok();
        });
    }

    // The size a terminal of `workspace` is attached at: the room, or the fitted slot or half the
    // room when split, never what another workspace's view fitted. None until the pane is laid out.
    fn attach_size(&self, workspace: &WorkspaceId, stacked: bool) -> Option<TerminalSize> {
        let split = self
            .layouts
            .get(workspace)
            .is_some_and(|layout| layout.shown.len() == 2);
        if split && let Some(slot) = self.slot {
            return Some(slot);
        }
        let room = self.room?;
        if !split {
            return Some(room);
        }
        if let Some((bounds, cell)) = self.room_pixels {
            let header = self.header_height.get().unwrap_or(Pixels::ZERO);
            let outer_width = bounds.width + SPACE_5 * 2.;
            let outer_height = bounds.height + SPACE_3 * 2.;
            let (width, height) = if stacked {
                (
                    bounds.width,
                    (outer_height - px(1.)) / 2. - header - SPACE_3,
                )
            } else {
                (
                    (outer_width - px(1.)) / 2. - SPACE_5 * 2.,
                    outer_height - header - SPACE_3,
                )
            };
            let columns = (f32::from(width) / f32::from(cell.width)).floor().max(1.) as u16;
            let rows = (f32::from(height) / f32::from(cell.height)).floor().max(1.) as u16;
            return Some(TerminalSize { columns, rows });
        }
        Some(if stacked {
            TerminalSize {
                rows: (room.rows / 2).max(1),
                ..room
            }
        } else {
            TerminalSize {
                columns: (room.columns / 2).max(1),
                ..room
            }
        })
    }

    fn room_found(
        &mut self,
        room: TerminalSize,
        bounds: Size<Pixels>,
        cell: Size<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.room != Some(room) {
            self.slot = None;
        }
        self.room = Some(room);
        self.room_pixels = Some((bounds, cell));
        self.attach_missing(cx);
    }

    fn attach_missing(&mut self, cx: &mut Context<Self>) {
        let stacked = self.models.settings.read(cx).terminal_place() != TerminalPlace::Below;
        let missing: Vec<TerminalInfo> = self
            .models
            .terminals
            .read(cx)
            .terminals()
            .iter()
            .filter(|info| self.entry(&info.id).is_none() && !self.attaching.contains_key(&info.id))
            .cloned()
            .collect();
        for info in missing {
            let Some(size) = self.attach_size(&info.workspace, stacked) else {
                continue;
            };
            self.attach(info, size, cx);
        }
    }

    fn attach(&mut self, info: TerminalInfo, size: TerminalSize, cx: &mut Context<Self>) {
        self.attaching
            .insert(info.id.clone(), Attaching { size, ended: None });
        let request = Request::AttachTerminal {
            terminal: info.id.clone(),
            size,
        };
        let asked = self
            .models
            .link
            .update(cx, |link, cx| link.ask(request, cx));
        cx.spawn(async move |pane, cx| {
            let answer = asked.await;
            pane.update(cx, |pane, cx| pane.attached(info, answer, cx))
                .ok();
        })
        .detach();
    }

    /// A refusal, as for a terminal that ended since it was listed, adds nothing: the shared list
    /// drops the terminal when told of its end, and the pane asks again only if the list changes.
    fn attached(
        &mut self,
        info: TerminalInfo,
        answer: Result<Reply, AskError>,
        cx: &mut Context<Self>,
    ) {
        let Some(Attaching { size, ended }) = self.attaching.remove(&info.id) else {
            return;
        };
        let Ok(Reply::Attached { cut }) = answer else {
            return;
        };
        if self.entry(&info.id).is_some() {
            return;
        }
        let view = self.make_entry(info.workspace.clone(), info.id.clone(), size, cut, cx);
        if let Some(ending) = ended {
            view.update(cx, |view, cx| view.ended(ending, cx));
        }
        let shown = self
            .layouts
            .entry(info.workspace.clone())
            .or_default()
            .offer(info.id);
        cx.notify();
        // A terminal that arrives takes the focus only from the pane's own handle, so the keys of a
        // Window that opens on its terminals go to the one it shows.
        if shown && self.on_screen.as_ref() == Some(&info.workspace) {
            let (window, own) = (self.window, self.focus.clone());
            cx.defer(move |cx| {
                window
                    .update(cx, |_, window, cx| {
                        if own.is_focused(window) {
                            view.read(cx).focus_handle().focus(window, cx);
                        }
                    })
                    .ok();
            });
        }
    }

    fn viewed(&mut self, workspace: &WorkspaceId, event: &ViewEvent) {
        let ViewEvent::Fitted(size) = event;
        self.size = *size;
        if self
            .layouts
            .get(workspace)
            .is_some_and(|layout| layout.shown.len() == 2)
        {
            self.slot = Some(*size);
        }
    }

    fn heard(&mut self, fact: &Fact, cx: &mut Context<Self>) {
        let Fact::TerminalEnded { terminal, ending } = fact else {
            return;
        };
        if let Some(view) = self.terminal(terminal) {
            view.update(cx, |view, cx| view.ended(*ending, cx));
        } else if let Some(attaching) = self.attaching.get_mut(terminal) {
            attaching.ended = Some(*ending);
        }
    }

    fn key_pressed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.contains_focused(window, cx) {
            return;
        }
        self.frames_until = Some(cx.background_executor().now() + FRAMES_AFTER_A_KEY);
        cx.notify();
    }

    fn click_slot(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(layout) = self.layout_mut()
            && n < layout.shown.len()
            && layout.focus_slot != n
        {
            layout.focus_slot = n;
            cx.notify();
        }
        self.focus_terminal(window, cx);
    }

    fn focus_terminal(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(entry) = self.focused().and_then(|id| self.entry(&id)) {
            entry.view.read(cx).focus_handle().focus(window, cx);
        }
    }

    fn hand_focus_on(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_terminal(window, cx);
    }

    fn show(&mut self, target: TerminalId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(layout) = self.layout_mut() {
            match layout.shown.iter().position(|shown| *shown == target) {
                Some(slot) => layout.focus_slot = slot,
                None => {
                    if let Some(slot) = layout.shown.get_mut(layout.focus_slot) {
                        *slot = target;
                    }
                }
            }
        }
        self.focus_terminal(window, cx);
        cx.notify();
    }

    fn neighbour(&self, by: usize) -> Option<TerminalId> {
        let focused = self.focused()?;
        let here = self.terminals_here();
        let at = here.iter().position(|id| *id == focused)?;
        Some(here[(at + by) % here.len()].clone())
    }

    fn next_terminal(&mut self, _: &NextTerminal, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(target) = self.neighbour(1) {
            self.show(target, window, cx);
        }
    }

    fn previous_terminal(
        &mut self,
        _: &PreviousTerminal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let last = self.terminals_here().len().wrapping_sub(1);
        if let Some(target) = self.neighbour(last) {
            self.show(target, window, cx);
        }
    }

    fn toggle_split(&mut self, _: &ToggleSplit, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminals_here().len() < 2 {
            return;
        }
        let Some(focused) = self.focused() else {
            return;
        };
        let shown = if self.split() {
            vec![focused]
        } else {
            let next = self.neighbour(1).unwrap_or_else(|| focused.clone());
            vec![focused, next]
        };
        if let Some(layout) = self.layout_mut() {
            layout.shown = shown;
            layout.focus_slot = 0;
        }
        self.focus_terminal(window, cx);
        cx.notify();
    }

    fn sync_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let given = self
            .focused()
            .and_then(|id| self.entry(&id))
            .and_then(|entry| entry.view.read(cx).title())
            .filter(|title| !title.is_empty());
        let wanted = match given {
            Some(title) => format!("{title} - YAAR"),
            None => "YAAR".to_owned(),
        };
        if self.window_title.as_ref() != Some(&wanted) {
            window.set_window_title(&wanted);
            self.window_title = Some(wanted);
        }
    }
}

fn how_to_open() -> String {
    let key = if cfg!(target_os = "macos") {
        "Cmd-T"
    } else {
        "Ctrl-T"
    };
    format!("No terminal is open. Press {key} to open one in the workspace on screen.")
}

impl Render for TerminalPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_title(window, cx);
        if self
            .frames_until
            .is_some_and(|until| cx.background_executor().now() < until)
        {
            window.request_animation_frame();
        }

        let palette = palette(cx);
        let stacked = self.models.settings.read(cx).terminal_place() != TerminalPlace::Below;
        let shown = self.shown();
        let split = shown.len() == 2;
        let launchers = self.render_launchers(cx);
        let consent = self.render_consent(cx);
        let slots = shown.iter().enumerate().filter_map(|(n, id)| {
            let entry = self.entry(id)?;
            let view = entry.view.clone();
            let name = entry.id.as_str().to_owned();
            let slot = div()
                .debug_selector(move || format!("terminal-slot-{n}"))
                .flex_1()
                .min_w_0()
                .min_h_0()
                .size_full()
                .on_mouse_down(
                    gpui_kit::MouseButton::Left,
                    cx.listener(move |pane, _, window, cx| pane.click_slot(n, window, cx)),
                );
            let mark = self.waiting(id, cx).then(|| {
                div().absolute().top(SPACE_4).right(SPACE_5).child(
                    Chip::new("Waiting for permission")
                        .tone(Tone::Warn)
                        .selector(format!("terminal-waiting-{n}")),
                )
            });
            let cut = entry
                .view
                .read(cx)
                .cut_band(window, cx)
                .map(|(top, height)| {
                    div()
                        .debug_selector(move || format!("terminal-cut-{n}"))
                        .absolute()
                        .top(top)
                        .left_0()
                        .right_0()
                        .h(height)
                        .border_b_1()
                        .border_color(palette.separator)
                });
            let divider = (n > 0).then(|| {
                if stacked {
                    Divider::horizontal().into_any_element()
                } else {
                    Divider::vertical().into_any_element()
                }
            });
            let header = split.then(|| {
                let measured = self.header_height.clone();
                let measure = canvas(
                    |bounds, _, _| bounds,
                    move |_, bounds, _, _| {
                        if bounds.size.height > Pixels::ZERO {
                            measured.set(Some(bounds.size.height));
                        }
                    },
                )
                .absolute()
                .size_full();
                div()
                    .flex_none()
                    .relative()
                    .pt(SPACE_2)
                    .bg(palette.pane)
                    .child(measure)
                    .child(
                        SectionLabel::new(self.terminal_label(id, cx))
                            .selector(format!("terminal-split-header-{n}")),
                    )
            });
            let named = div()
                .debug_selector(move || format!("terminal-{name}"))
                .size_full()
                .child(view);
            let attached = self.render_attachment(n, id, cx);
            let slot_body = slot
                .relative()
                .child(named)
                .children(cut)
                .children(mark)
                .children(attached);
            let slot_wrap = if split {
                div().flex_1().min_w_0().min_h_0().px(SPACE_5).pb(SPACE_3)
            } else {
                div().flex_1().min_w_0().min_h_0().px(SPACE_5).py(SPACE_3)
            };
            // The header and inset padding are siblings/ancestors of `terminal-slot-{n}`, so the
            // slot's bounds stay exactly those of its terminal view.
            let cell = div()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .flex()
                .flex_col()
                .children(header)
                .child(slot_wrap.child(slot_body))
                .into_any_element();
            Some((divider, cell))
        });
        let slots = slots.flat_map(|(divider, slot)| divider.into_iter().chain([slot]));
        let empty = shown.is_empty().then(|| {
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    EmptyState::new("No terminal open")
                        .line(how_to_open())
                        .selector("terminal-how-to-open"),
                )
        });
        let slots_area = (!shown.is_empty()).then(|| {
            let area = div().flex_1().min_h_0().flex();
            let area = if stacked {
                area.flex_col()
            } else {
                area.flex_row()
            };
            area.children(slots)
        });
        let cell = TerminalView::metrics(window, cx).2;
        let (weak, seen) = (cx.weak_entity(), self.room_seen.clone());
        let room = div().absolute().inset_0().px(SPACE_5).py(SPACE_3).child(
            canvas(
                |bounds, _, _| bounds,
                move |_, bounds, window, cx| {
                    let columns =
                        (f32::from(bounds.size.width) / f32::from(cell.width)).floor() as u16;
                    let rows =
                        (f32::from(bounds.size.height) / f32::from(cell.height)).floor() as u16;
                    let room = TerminalSize { columns, rows };
                    if columns == 0 || rows == 0 || seen.get() == Some(room) {
                        return;
                    }
                    seen.set(Some(room));
                    let weak = weak.clone();
                    let size = bounds.size;
                    window.defer(cx, move |_, cx| {
                        weak.update(cx, |pane, cx| pane.room_found(room, size, cell, cx))
                            .ok();
                    });
                },
            )
            .size_full(),
        );
        let notice = self
            .notice
            .clone()
            .map(|notice| Banner::new(notice).tone(Tone::Danger));
        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .bg(palette.pane)
            .flex()
            .flex_col()
            .children(launchers)
            .children(consent)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .flex()
                    .flex_col()
                    .child(room)
                    .children(notice)
                    .children(empty)
                    .children(slots_area),
            )
    }
}

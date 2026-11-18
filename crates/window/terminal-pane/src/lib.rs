//! Terminal pane (`W-TERM`).
//!
//! Owns: the terminal emulator and its drawing, several terminals, launchers, the attachment shown at the agent's terminal.
//!
//! Surface:
//! - [`entry`], the pane as the Window program registers it, with [`pane`], which builds
//!   it for the Frame, and [`controls`], its actions with their keys;
//! - [`TerminalPane`], the view of the pane and the model of what it shows: its
//!   terminals, which are shown, which has the focus; and [`TerminalView`], one terminal;
//! - [`Emulator`], the screen of one terminal as its program's output builds it, with
//!   the answers the terminal owes the program;
//! - [`encode_key`], [`encode_release`], [`encode_paste`], [`encode_focus`] and
//!   [`encode_mouse`], what input sends to a program in each state of its terminal, with
//!   [`Modes`], [`KeyboardFlags`], [`IMPLEMENTED_FLAGS`] and [`OptionKey`].
//!
//! The pane asks the Keeper, through the Frame's link, to open a terminal for the
//! workspace on screen; it sends the program what is typed and draws what the program
//! writes. It holds several terminals and shows one, or two side by side in the split
//! view. The Keeper owns the terminals and their programs: closing the window ends none.
//!
//! It depends on the Frame and on no other pane.

mod draw;
mod emulator;
mod keys;
mod modes;
mod mouse;
mod view;

use std::time::{Duration, Instant};

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;
use yaar_frame::{AskError, Asks, BuildPane, Control, Models, Pane, PaneEntry, Place, Sends};
use yaar_link::{
    Fact, Program, Reply, Request, RequestKind, TerminalId, TerminalSize, WorkspaceId,
};

pub use emulator::{Cell, CellPlace, Color, Cursor, Emulator, HISTORY_LINES, Screen, Style};
pub use keys::{OptionKey, encode_focus, encode_key, encode_paste, encode_release};
pub use modes::{IMPLEMENTED_FLAGS, KeyboardFlags, Modes, MouseMode};
pub use mouse::{MouseAction, MouseButton, encode_mouse};
pub use view::{TerminalView, ViewEvent};

actions!(
    terminal,
    [
        /// Opens a new terminal for the workspace on screen, with the user's shell in it,
        /// and gives it the focus.
        NewTerminal,
        /// Shows the next terminal, in the order they were opened, and from the last to
        /// the first, and gives it the focus.
        NextTerminal,
        /// Shows the previous terminal, and from the first to the last, and gives it the
        /// focus.
        PreviousTerminal,
        /// Shows two terminals side by side, or one again.
        ToggleSplit,
        /// Copies the text selected in the terminal with the focus.
        Copy,
        /// Sends the text of the clipboard to the program of the terminal with the focus,
        /// as a paste.
        Paste,
        /// Scrolls the history of the terminal with the focus one screen up.
        ScrollPageUp,
        /// Scrolls the history of the terminal with the focus one screen down.
        ScrollPageDown,
    ]
);

/// The name of the pane in the Frame.
pub const NAME: &str = "terminal";

/// The key context of the pane, wherever the focus is inside it.
pub const KEY_CONTEXT: &str = "TerminalPane";

/// The size a terminal is opened at before the pane has been laid out.
pub const FIRST_SIZE: TerminalSize = TerminalSize {
    columns: 80,
    rows: 24,
};

/// How long after a key the pane keeps asking for a frame at every frame: a program's
/// answer to the key is drawn in the first frame after it arrives, not when something
/// else asks for one.
pub const FRAMES_AFTER_A_KEY: Duration = Duration::from_secs(1);

/// The pane as the Window program registers it: its name, its keys and its builder.
pub fn entry() -> PaneEntry {
    PaneEntry {
        name: NAME,
        controls,
        build: pane,
    }
}

/// Builds the Terminal pane for a window, under the name [`NAME`].
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

/// The actions of the pane with their keys.
///
/// [`NewTerminal`], [`NextTerminal`], [`PreviousTerminal`] and [`ToggleSplit`] work
/// anywhere, on `secondary-t`, `secondary-]`, `secondary-[` and `secondary-d`;
/// [`NewTerminal`] sends [`RequestKind::OpenTerminal`]. [`Copy`], [`Paste`],
/// [`ScrollPageUp`] and [`ScrollPageDown`] are kept keys: they work inside a terminal
/// only, on a key no program is given. Copy and paste are Cmd-C and Cmd-V on macOS and
/// Ctrl-Shift-C and Ctrl-Shift-V elsewhere.
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
    vec![
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
    ]
}

/// One terminal of the pane, with what the pane keeps beside its view.
struct Entry {
    id: TerminalId,
    /// The workspace the terminal was opened for.
    workspace: WorkspaceId,
    view: Entity<TerminalView>,
    _events: Subscription,
}

/// The Terminal pane: the terminals the Window opened, the one or two it shows, and the
/// one that has the focus.
///
/// It takes [`NewTerminal`], [`NextTerminal`], [`PreviousTerminal`] and [`ToggleSplit`]
/// from wherever the focus is in the window, through the Frame's routes. Each terminal
/// keeps the workspace it was opened for.
///
/// A new terminal is added after the others, shown in the slot that has the focus, and
/// given the focus. In the split view the pane draws two slots side by side, the
/// terminal that has the focus and the next one in the list; [`NextTerminal`] and
/// [`PreviousTerminal`] move the focus to the next or previous terminal of the list, and
/// from the last to the first: into the other slot when that terminal is shown there,
/// else into the slot that has the focus, which then shows it. A click in a slot gives
/// it the focus. A terminal whose program ended stays in the list with its last screen.
/// Each slot is drawn inside an element with the selector `terminal-slot-<n>`, counted
/// from 0 from the left, which `debug_bounds` finds while it is drawn.
///
/// When the pane's own focus handle gains the focus, as when the Frame moves the focus
/// from pane to pane, the pane hands it on to the view of [`TerminalPane::focused`], so
/// that the keys reach that terminal; with no terminal the focus stays on the pane.
///
/// After every key typed while the focus is inside the pane, a key that runs an action
/// included, the pane asks for a frame at every frame for [`FRAMES_AFTER_A_KEY`]. It
/// learns of each key through a keystroke interceptor, which sees a key before any
/// binding or key listener takes it.
///
/// The window's title is `YAAR` until the program of the focused terminal gives a title,
/// and then that title followed by ` - YAAR`; it follows the focused terminal.
pub struct TerminalPane {
    models: Models,
    focus: FocusHandle,
    window: AnyWindowHandle,
    /// The size the pane last laid a terminal out at, which a new terminal is opened at.
    size: TerminalSize,
    /// Why the last terminal asked for was not opened, until the next is asked for.
    notice: Option<String>,
    terminals: Vec<Entry>,
    /// The terminals drawn, left to right: one, or two in the split view.
    shown: Vec<TerminalId>,
    /// Which of the slots has the focus.
    focus_slot: usize,
    /// Until when each frame asks for the next one: the end of the second after the
    /// last key.
    frames_until: Option<Instant>,
    /// The title the pane last gave the window.
    window_title: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl TerminalPane {
    /// An empty pane of the window whose models are `models`.
    pub fn new(models: &Models, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let pane = cx.entity();
        models.routes.route(&pane, Self::new_terminal);
        models.routes.route(&pane, Self::next_terminal);
        models.routes.route(&pane, Self::previous_terminal);
        models.routes.route(&pane, Self::toggle_split);

        let focus = cx.focus_handle();
        let handed_on = cx.on_focus(&focus, window, |pane, window, cx| {
            pane.hand_focus_on(window, cx);
        });
        // A key that runs an action never reaches a key listener, so the keys are
        // counted here, before any binding takes them.
        let weak = cx.weak_entity();
        let keys = cx.intercept_keystrokes(move |_, window, cx| {
            weak.update(cx, |pane, cx| pane.key_pressed(window, cx))
                .ok();
        });
        let facts = cx.subscribe(&models.link, |pane, _, fact: &Fact, cx| {
            pane.heard(fact, cx);
        });
        Self {
            models: models.clone(),
            focus,
            window: window.window_handle(),
            size: FIRST_SIZE,
            notice: None,
            terminals: Vec::new(),
            shown: Vec::new(),
            focus_slot: 0,
            frames_until: None,
            window_title: None,
            _subscriptions: vec![handed_on, keys, facts],
        }
    }

    /// The terminals of the pane, in the order they were opened, those whose program
    /// ended included.
    pub fn terminals(&self) -> Vec<TerminalId> {
        self.terminals
            .iter()
            .map(|entry| entry.id.clone())
            .collect()
    }

    /// The view of one terminal of the pane.
    pub fn terminal(&self, id: &TerminalId) -> Option<Entity<TerminalView>> {
        self.entry(id).map(|entry| entry.view.clone())
    }

    /// The workspace a terminal of the pane was opened for.
    pub fn workspace(&self, id: &TerminalId) -> Option<WorkspaceId> {
        self.entry(id).map(|entry| entry.workspace.clone())
    }

    /// The terminals drawn now, left to right: none, one, or two in the split view.
    pub fn shown(&self) -> Vec<TerminalId> {
        self.shown.clone()
    }

    /// Whether the split view is on.
    pub fn split(&self) -> bool {
        self.shown.len() == 2
    }

    /// The terminal the keys go to while the pane has the focus: one of those shown.
    pub fn focused(&self) -> Option<TerminalId> {
        self.shown.get(self.focus_slot).cloned()
    }

    /// Why the last terminal asked for was not opened; none once another is asked for.
    /// The pane draws this very sentence.
    pub fn notice(&self) -> Option<String> {
        self.notice.clone()
    }

    fn entry(&self, id: &TerminalId) -> Option<&Entry> {
        self.terminals.iter().find(|entry| entry.id == *id)
    }

    /// Asks the Keeper for a new terminal for the workspace on screen, at the size the
    /// pane lays terminals out at. With no workspace on screen there is nothing to open
    /// one in, and nothing is asked. Every question waits for its own answer: a second
    /// terminal asked for before the first answer came loses neither.
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
        // Detached, not kept: a task that is dropped is cancelled, and its answer lost.
        // The pane is held weakly, so a pane that is gone takes no answer.
        cx.spawn(async move |pane, cx| {
            let answer = asked.await;
            pane.update(cx, |pane, cx| pane.opened(workspace, answer, cx))
                .ok();
        })
        .detach();
        cx.notify();
    }

    /// Takes the Keeper's answer to the question for a new terminal of `workspace`.
    fn opened(
        &mut self,
        workspace: WorkspaceId,
        answer: Result<Reply, AskError>,
        cx: &mut Context<Self>,
    ) {
        match answer {
            Ok(Reply::TerminalOpened { terminal }) => self.add(workspace, terminal, cx),
            Ok(_) => self.notice = Some(AskError::Unexpected.to_string()),
            Err(error) => self.notice = Some(error.to_string()),
        }
        cx.notify();
    }

    /// Adds a terminal the Keeper opened: it takes the output at once, so that none of
    /// what the program wrote since the reply is lost, shows in the slot that has the
    /// focus and takes the focus.
    fn add(&mut self, workspace: WorkspaceId, id: TerminalId, cx: &mut Context<Self>) {
        if self.entry(&id).is_some() {
            return;
        }
        let link = self.models.link.clone();
        let output = link.update(cx, |link, _| link.take_output(&id));
        let size = self.size;
        let view = cx.new(|cx| TerminalView::new(id.clone(), size, link, output, cx));
        let events = cx.subscribe(&view, |pane, _, event: &ViewEvent, _| {
            pane.viewed(event);
        });
        self.terminals.push(Entry {
            id: id.clone(),
            workspace,
            view: view.clone(),
            _events: events,
        });
        match self.shown.get_mut(self.focus_slot) {
            Some(slot) => *slot = id,
            None => {
                self.shown.push(id);
                self.focus_slot = self.shown.len() - 1;
            }
        }
        // There is no window in what answers the Keeper: the focus is given once the
        // answer is taken.
        let window = self.window;
        cx.defer(move |cx| {
            window
                .update(cx, |_, window, cx| {
                    view.read(cx).focus_handle().focus(window, cx)
                })
                .ok();
        });
    }

    /// What one of the terminal views tells.
    fn viewed(&mut self, event: &ViewEvent) {
        let ViewEvent::Fitted(size) = event;
        self.size = *size;
    }

    /// What the Keeper tells that the pane shows: a program that ended.
    fn heard(&mut self, fact: &Fact, cx: &mut Context<Self>) {
        if let Fact::TerminalEnded { terminal, ending } = fact
            && let Some(view) = self.terminal(terminal)
        {
            view.update(cx, |view, cx| view.ended(*ending, cx));
        }
    }

    /// A key was typed somewhere in the window: while the focus is in the pane, the next
    /// second of frames is asked for.
    fn key_pressed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.contains_focused(window, cx) {
            return;
        }
        self.frames_until = Some(cx.background_executor().now() + FRAMES_AFTER_A_KEY);
        cx.notify();
    }

    /// A click in slot `n`: its terminal has the focus.
    fn click_slot(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        if n < self.shown.len() && self.focus_slot != n {
            self.focus_slot = n;
            cx.notify();
        }
        self.focus_terminal(window, cx);
    }

    /// Gives the focus to the view of the terminal that has it.
    fn focus_terminal(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(entry) = self.focused().and_then(|id| self.entry(&id)) {
            entry.view.read(cx).focus_handle().focus(window, cx);
        }
    }

    /// The pane's own handle gained the focus: the keys are the focused terminal's.
    fn hand_focus_on(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_terminal(window, cx);
    }

    /// Shows `target` in the slot that has the focus, or moves the focus to the slot
    /// that shows it, and gives the focus to its view.
    fn show(&mut self, target: TerminalId, window: &mut Window, cx: &mut Context<Self>) {
        match self.shown.iter().position(|shown| *shown == target) {
            Some(slot) => self.focus_slot = slot,
            None => {
                if let Some(slot) = self.shown.get_mut(self.focus_slot) {
                    *slot = target;
                }
            }
        }
        self.focus_terminal(window, cx);
        cx.notify();
    }

    /// The terminal `by` places after the focused one in the list, round at the ends.
    fn neighbour(&self, by: usize) -> Option<TerminalId> {
        let focused = self.focused()?;
        let at = self
            .terminals
            .iter()
            .position(|entry| entry.id == focused)?;
        let n = self.terminals.len();
        Some(self.terminals[(at + by) % n].id.clone())
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
        let last = self.terminals.len().wrapping_sub(1);
        if let Some(target) = self.neighbour(last) {
            self.show(target, window, cx);
        }
    }

    fn toggle_split(&mut self, _: &ToggleSplit, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminals.len() < 2 {
            return;
        }
        let Some(focused) = self.focused() else {
            return;
        };
        self.shown = if self.split() {
            vec![focused]
        } else {
            let next = self.neighbour(1).unwrap_or_else(|| focused.clone());
            vec![focused, next]
        };
        self.focus_slot = 0;
        self.focus_terminal(window, cx);
        cx.notify();
    }

    /// Gives the window the title of the focused terminal, when it is not the one the
    /// window has.
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

/// What the pane says while it holds no terminal: how to open one.
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
        // After a key the answer of the program is drawn in the first frame it arrives
        // in, not whenever something else asks for one.
        if self
            .frames_until
            .is_some_and(|until| cx.background_executor().now() < until)
        {
            window.request_animation_frame();
        }

        let theme = cx.theme();
        let (border, muted) = (theme.border, theme.muted_foreground);
        let slots = self.shown.iter().enumerate().filter_map(|(n, id)| {
            let view = self.entry(id)?.view.clone();
            // A click in a slot gives it the focus; its view takes the focus itself, and
            // the pane learns which slot has it here, whatever GPUI tells of the focus.
            let slot = div()
                .debug_selector(move || format!("terminal-slot-{n}"))
                .flex_1()
                .min_w_0()
                .h_full()
                .on_mouse_down(
                    gpui_kit::MouseButton::Left,
                    cx.listener(move |pane, _, window, cx| pane.click_slot(n, window, cx)),
                );
            // The divider is a sibling of the slot, so the slot's bounds are exactly its
            // view's.
            let divider = (n > 0).then(|| div().flex_none().w_px().h_full().bg(border));
            Some((divider, slot.child(view)))
        });
        let slots = slots.flat_map(|(divider, slot)| divider.into_iter().chain([slot]));
        let empty = self
            .shown
            .is_empty()
            .then(|| div().p_3().text_sm().text_color(muted).child(how_to_open()));
        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .children(self.notice.clone().map(|notice| {
                div()
                    .flex_none()
                    .px_3()
                    .py_1()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(notice)
            }))
            .children(empty)
            .child(div().flex_1().min_h_0().flex().flex_row().children(slots))
    }
}

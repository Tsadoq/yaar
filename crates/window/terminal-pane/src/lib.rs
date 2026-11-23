//! Terminal pane (`W-TERM`).
//!
//! Owns: the terminal emulator and its drawing, several terminals, launchers, the attachment shown at the agent's terminal.
//!
//! Next to the shell it offers a launcher for each agent CLI the Keeper names, and a
//! control that resumes each session a restart cut off (`launchers.rs`). Under them it
//! shows the question an Adapter asks before it writes into the user's own settings of its
//! CLI, with the two controls that answer it (`consent.rs`).
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

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;
use yaar_frame::{AskError, Asks, BuildPane, Control, Models, Pane, PaneEntry, Place, Sends};
use yaar_link::{
    Ending, Fact, Program, Reply, Request, RequestKind, SessionState, TerminalId, TerminalInfo,
    TerminalSize, WorkspaceId,
};

pub use attachment::ADDRESS_EACH_COMMENT;
pub use consent::{NO_LABEL, NOTHING_TO_INSTALL, YES_LABEL};
pub use emulator::{
    Cell, CellPlace, Color, Cursor, Emulator, HISTORY_LINES, Progress, Screen, Style,
};
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
        /// Shows, over the agent's terminal with the focus, what the next message will
        /// carry of the review attached to it, as the agent will receive it; again, hides
        /// it.
        ViewAttachment,
        /// Takes away the review attached to the next message of the agent of the
        /// terminal with the focus.
        DetachAttachment,
        /// Types the sentence "Address each comment." at the prompt of the agent of the
        /// terminal with the focus, while a review is attached to its next message and it
        /// shows no permission prompt; the user presses Enter.
        AddressEachComment,
        /// Opens a new terminal for the workspace on screen that runs the launcher of the
        /// chosen agent CLI, and gives it the focus.
        NewAgentTerminal,
        /// Makes the next agent CLI the Keeper names the chosen one, whose launcher
        /// New Agent Terminal opens; after the last, the first.
        NextAgentCli,
        /// Opens a terminal that resumes the next session of the workspace on screen that
        /// a restart cut off, with its CLI's own command.
        ResumeAgent,
        /// Answers yes to the question of every Adapter that writes into its CLI's own
        /// settings only with the user's leave: each installs its entry.
        InstallAgentExtensions,
        /// Answers no to that question: nothing is installed, and an entry installed
        /// before is taken out.
        KeepAgentExtensionsOut,
        /// Empties the history of the terminal with the focus and every line of its screen
        /// above the cursor's, which becomes the first: nothing is sent to the program.
        Clear,
        /// Selects the whole history and screen of the terminal with the focus.
        SelectAll,
        /// Scrolls the history of the terminal with the focus to its first line.
        ScrollToTop,
        /// Scrolls it back to the newest line.
        ScrollToBottom,
        /// Scrolls it one line up.
        ScrollLineUp,
        /// Scrolls it one line down.
        ScrollLineDown,
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

/// What the pane's notice says while a worktree made with an agent is being made, before the
/// branch's name.
pub const WORKTREE_BEING_MADE: &str = "Making the worktree of";

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
/// Ctrl-Shift-C and Ctrl-Shift-V elsewhere. [`NewAgentTerminal`] (`secondary-shift-t`),
/// [`NextAgentCli`] (`secondary-alt-c`) and [`ResumeAgent`] (`secondary-shift-r`) work
/// anywhere; the first and the last send [`RequestKind::OpenTerminal`], of a launcher and
/// of a session's resume. [`InstallAgentExtensions`] (`secondary-alt-e`) and
/// [`KeepAgentExtensionsOut`] (`secondary-alt-shift-e`) work anywhere and send
/// [`RequestKind::AnswerConsent`], yes and no to an Adapter's question for leave.
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
            // Not Ctrl-Alt-T, which the Linux desktops give to opening a terminal of
            // their own before the app sees it.
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
    // The Mac terminals' keys. On Linux only Ctrl-Shift-C and Ctrl-Shift-V are kept in a
    // terminal, and every other key reaches the program.
    #[cfg(target_os = "macos")]
    table.extend([
        nothing("Clear", || Box::new(Clear), "cmd-k", in_a_terminal),
        nothing("Select All", || Box::new(SelectAll), "cmd-a", in_a_terminal),
        nothing(
            "Scroll to Top",
            || Box::new(ScrollToTop),
            "cmd-home",
            in_a_terminal,
        ),
        nothing(
            "Scroll to Bottom",
            || Box::new(ScrollToBottom),
            "cmd-end",
            in_a_terminal,
        ),
        nothing(
            "Scroll Line Up",
            || Box::new(ScrollLineUp),
            "cmd-up",
            in_a_terminal,
        ),
        nothing(
            "Scroll Line Down",
            || Box::new(ScrollLineDown),
            "cmd-down",
            in_a_terminal,
        ),
    ]);
    table
}

/// One terminal of the pane, with what the pane keeps beside its view.
struct Entry {
    id: TerminalId,
    /// The workspace the terminal was opened for.
    workspace: WorkspaceId,
    view: Entity<TerminalView>,
    /// Whether the Keeper had dropped part of the output when the pane attached to it.
    cut: bool,
    _events: Subscription,
}

/// A terminal the pane asked the Keeper to attach, until the answer is taken.
struct Attaching {
    /// The size the request gave the Keeper, which the view starts at.
    size: TerminalSize,
    /// How the program ended, if the Keeper told that before the answer came.
    ended: Option<Ending>,
}

/// What a workspace shows of its terminals: the one or two drawn, left to right, and which
/// of the slots has the focus. A workspace keeps its own while another is on screen.
#[derive(Default)]
struct Layout {
    shown: Vec<TerminalId>,
    focus_slot: usize,
}

impl Layout {
    /// Shows a terminal that was just opened in the slot that has the focus, or in a new
    /// slot when none is shown, and gives that slot the focus.
    fn put(&mut self, id: TerminalId) {
        match self.shown.get_mut(self.focus_slot) {
            Some(slot) => *slot = id,
            None => {
                self.shown.push(id);
                self.focus_slot = self.shown.len() - 1;
            }
        }
    }

    /// Shows a terminal the pane attached to, unless the workspace shows one already: it
    /// takes no slot from what the user is looking at. Whether it was shown.
    fn offer(&mut self, id: TerminalId) -> bool {
        let empty = self.shown.is_empty();
        if empty {
            self.put(id);
        }
        empty
    }

    /// The terminal that has the focus.
    fn focused(&self) -> Option<&TerminalId> {
        self.shown.get(self.focus_slot)
    }
}

/// The Terminal pane: the terminals the Window opened, the one or two it shows, and the
/// one that has the focus.
///
/// It takes [`NewTerminal`], [`NextTerminal`], [`PreviousTerminal`] and [`ToggleSplit`]
/// from wherever the focus is in the window, through the Frame's routes. Each terminal
/// keeps the workspace it was opened for.
///
/// The pane shows the terminals of the workspace on screen and no others. Each workspace
/// keeps which of its terminals had the focus and whether it was split, and has them again
/// when it comes on screen again; the terminals of the others go on running, and
/// [`TerminalPane::terminals`] lists every one the window opened. With no terminal of the
/// workspace on screen the pane says how to open one, under the selector
/// `terminal-how-to-open`, and the focus stays on its own handle.
///
/// A terminal the Keeper holds that the Window did not open, listed by
/// `Models::terminals` when the link connects or whenever the list gains one, is
/// attached: the pane asks the Keeper to attach it at the size its own workspace would
/// show it at: the grid of the room the pane itself has (so an attach at connect waits
/// for the pane's first frame) when the workspace shows one terminal or none, a slot's
/// grid when it is split, and never the size a view of another workspace fitted last. The
/// view starts at that grid and tells the Keeper nothing more while it fits. The output
/// that comes after the answer, the kept output first and then what the program writes,
/// is read as the output of a terminal the pane opened is. It is listed beside the
/// others, with the workspace the Keeper says; it is shown when its workspace shows no
/// other terminal, and never takes a slot from one. A terminal the pane opens joins that
/// list itself, and is not attached. When the Keeper had dropped the oldest output
/// ([`TerminalPane::attached_cut`]) the first line the view holds says that older output
/// is gone, and the mark under the selector `terminal-cut-<its slot>` is drawn over that
/// line for as long as it is in view.
///
/// A new terminal is added after the others, shown in the slot that has the focus, and
/// given the focus, when its workspace is on screen; one whose answer comes after another
/// workspace came on screen is shown when its own returns. In the split view the pane
/// draws two slots side by side, the terminal that has the focus and the next one of the
/// workspace; [`NextTerminal`] and [`PreviousTerminal`] move the focus to the next or
/// previous terminal of the workspace on screen, and from the last to the first: into the
/// other slot when that terminal is shown there, else into the slot that has the focus,
/// which then shows it. A click in a slot gives it the focus. A terminal whose program
/// ended stays in the list with its last screen.
/// Each slot is drawn inside an element with the selector `terminal-slot-<n>`, counted
/// from 0 from the left, which `debug_bounds` finds while it is drawn; inside it the view
/// of the terminal is drawn inside an element with the selector `terminal-<id>`.
///
/// When the pane's own focus handle gains the focus, as when the Frame moves the focus
/// from pane to pane, the pane hands it on to the view of [`TerminalPane::focused`], so
/// that the keys reach that terminal; with no terminal the focus stays on the pane. When
/// another workspace comes on screen while the focus is inside the pane, it goes to that
/// workspace's focused terminal, or to the pane's own handle when it has none.
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
    /// The size the last view fitted while its workspace showed two terminals side by
    /// side: what a slot holds. A view that fits the size it was started at tells nothing,
    /// so this can stay unknown while terminals are laid out. Forgotten when the room
    /// changes, as a slot fitted in another room holds something else.
    slot: Option<TerminalSize>,
    /// The grid the room the pane has holds, as the painter found it; none before the
    /// first frame.
    room: Option<TerminalSize>,
    /// The room the painter last told the pane of, so that a frame that finds the same
    /// tells nobody anything.
    room_seen: Rc<std::cell::Cell<Option<TerminalSize>>>,
    /// Why the last terminal asked for was not opened, until the next is asked for.
    notice: Option<String>,
    terminals: Vec<Entry>,
    /// The workspace on screen, as the shared list last said.
    on_screen: Option<WorkspaceId>,
    /// What each workspace shows of its terminals, once it showed any.
    layouts: HashMap<WorkspaceId, Layout>,
    /// Until when each frame asks for the next one: the end of the second after the
    /// last key.
    frames_until: Option<Instant>,
    /// The title the pane last gave the window.
    window_title: Option<String>,
    /// The terminals the pane asked the Keeper to attach and has no answer for.
    attaching: HashMap<TerminalId, Attaching>,
    /// What the pane shows of the reviews attached to its agents' next messages: its own
    /// state, chosen by the package that builds it (`attachment.rs`).
    attachments: attachment::Attachments,
    /// The chosen launcher and the resumes opened: the state of `launchers.rs`.
    launchers: launchers::Launchers,
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
        let workspaces = cx.observe_in(&models.workspaces, window, |pane, _, window, cx| {
            pane.workspace_changed(window, cx);
        });
        // The waiting mark follows the agents, whatever else changes.
        let agents = cx.observe(&models.agents, |_, _, cx| cx.notify());
        // The question for leave comes and goes with what the Keeper tells.
        let consents = cx.observe(&models.consents, |_, _, cx| cx.notify());
        // A terminal the list gains is attached.
        let terminals = cx.observe(&models.terminals, |pane, _, cx| pane.attach_missing(cx));
        // A terminal another pane wants is opened here.
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
            room_seen: Rc::default(),
            notice: None,
            terminals: Vec::new(),
            on_screen,
            layouts: HashMap::new(),
            frames_until: None,
            window_title: None,
            attaching: HashMap::new(),
            attachments: attachment::Attachments::default(),
            launchers: launchers::Launchers::default(),
            _subscriptions: vec![
                handed_on, keys, facts, workspaces, agents, consents, terminals, wanted,
            ],
        };
        pane.attach_missing(cx);
        pane
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

    /// The terminals drawn now, left to right: none, one, or two in the split view. All
    /// are terminals of the workspace on screen.
    pub fn shown(&self) -> Vec<TerminalId> {
        self.layout()
            .map(|layout| layout.shown.clone())
            .unwrap_or_default()
    }

    /// Whether the split view is on, for the workspace on screen.
    pub fn split(&self) -> bool {
        self.layout().is_some_and(|layout| layout.shown.len() == 2)
    }

    /// The terminal the keys go to while the pane has the focus: one of those shown.
    pub fn focused(&self) -> Option<TerminalId> {
        self.layout()?.focused().cloned()
    }

    /// Why the last terminal asked for was not opened; none once another is asked for.
    /// The pane draws this very sentence.
    pub fn notice(&self) -> Option<String> {
        self.notice.clone()
    }

    /// Whether `terminal` carries the waiting mark: an agent session runs in it and waits
    /// at a permission prompt for the user, as `yaar_frame::AgentList` says. A terminal
    /// that is drawn and carries it draws the mark under the selector
    /// `terminal-waiting-<its slot>`, beside `terminal-slot-<its slot>`. The pane sends
    /// the program nothing for it.
    pub fn waiting(&self, terminal: &TerminalId, cx: &App) -> bool {
        self.models
            .agents
            .read(cx)
            .in_terminal(terminal)
            .is_some_and(|session| session.state == SessionState::WaitsForPermission)
    }

    /// Whether the pane attached to `terminal` after the Keeper had dropped part of its
    /// output: the view then draws, where the kept output starts, a line that says older
    /// output is gone, under the selector `terminal-cut-<its slot>`. False for a terminal
    /// this Window opened, and for one the Keeper kept whole.
    pub fn attached_cut(&self, terminal: &TerminalId) -> bool {
        self.entry(terminal).is_some_and(|entry| entry.cut)
    }

    fn entry(&self, id: &TerminalId) -> Option<&Entry> {
        self.terminals.iter().find(|entry| entry.id == *id)
    }

    /// What the workspace on screen shows; none before it showed any terminal.
    fn layout(&self) -> Option<&Layout> {
        self.layouts.get(self.on_screen.as_ref()?)
    }

    /// The layout of the workspace on screen, made when it has none. None while no
    /// workspace is on screen.
    fn layout_mut(&mut self) -> Option<&mut Layout> {
        Some(self.layouts.entry(self.on_screen.clone()?).or_default())
    }

    /// The terminals of the workspace on screen, in the order they were opened.
    fn terminals_here(&self) -> Vec<TerminalId> {
        self.terminals
            .iter()
            .filter(|entry| Some(&entry.workspace) == self.on_screen.as_ref())
            .map(|entry| entry.id.clone())
            .collect()
    }

    /// Another workspace came on screen, or none: what the pane shows is that one's. When
    /// the focus was inside the pane it follows, so that the keys do not go to a terminal
    /// that is no longer drawn.
    fn workspace_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = self.models.workspaces.read(cx).selected().cloned();
        if now == self.on_screen {
            return;
        }
        // Read before the frame that leaves the old terminal is drawn.
        let had_the_focus = self.focus.contains_focused(window, cx);
        self.on_screen = now;
        if had_the_focus {
            if self.focused().is_some() {
                self.focus_terminal(window, cx);
            } else {
                self.focus.focus(window, cx);
            }
        }
        cx.notify();
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
            pane.update(cx, |pane, cx| {
                pane.opened(workspace, Program::Shell, answer, cx)
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Takes the Keeper's answer to the question for a new terminal of `workspace` that
    /// runs `program`.
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

    /// Makes the entry of a terminal whose grid the Keeper holds at `size`: its view takes
    /// the output at once, so that none of what the program wrote since the reply is lost.
    /// Not yet shown.
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

    /// Adds a terminal the Keeper opened for this Window to run `program`: it is shown in
    /// the slot that has the focus and takes the focus, and joins the shared list of
    /// terminals, as that program, which does not attach to it.
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
        // A terminal of a workspace that is not on screen takes nothing from the one that
        // is: it is shown when its own comes back.
        if self.on_screen.as_ref() != Some(&workspace) {
            return;
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

    /// The size a terminal of `workspace` is attached at: the size its own workspace
    /// would show it at, whatever another workspace on screen showed last. A workspace
    /// that is split shows it in a slot, which is the size the last view fitted in a
    /// split; any other shows it alone, filling the room. With no frame yet the room is
    /// not known, and an attach waits for it.
    fn attach_size(&self, workspace: &WorkspaceId) -> Option<TerminalSize> {
        let split = self
            .layouts
            .get(workspace)
            .is_some_and(|layout| layout.shown.len() == 2);
        if split && let Some(slot) = self.slot {
            return Some(slot);
        }
        let room = self.room?;
        Some(if split {
            // No view fitted a slot yet: half the room, which the view corrects once.
            TerminalSize {
                columns: room.columns / 2,
                ..room
            }
        } else {
            room
        })
    }

    /// The painter found how many columns and rows the room the pane has holds: the
    /// terminals that waited for it are attached.
    fn room_found(&mut self, room: TerminalSize, cx: &mut Context<Self>) {
        if self.room != Some(room) {
            // A slot fitted in another room is not what a slot holds now: half the room
            // answers until a split view fits again.
            self.slot = None;
        }
        self.room = Some(room);
        self.attach_missing(cx);
    }

    /// Asks the Keeper to attach every terminal the shared list holds that the pane has
    /// none of and is not already attaching, once the size to attach at is known.
    fn attach_missing(&mut self, cx: &mut Context<Self>) {
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
            let Some(size) = self.attach_size(&info.workspace) else {
                continue;
            };
            self.attach(info, size, cx);
        }
    }

    /// Asks the Keeper to attach one terminal, at the size the pane shows terminals at.
    /// The size goes once, in the request: the Keeper gives it to the program.
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
        // Detached, as the question for a new terminal is: a task that is dropped is
        // cancelled, and its answer lost.
        cx.spawn(async move |pane, cx| {
            let answer = asked.await;
            pane.update(cx, |pane, cx| pane.attached(info, answer, cx))
                .ok();
        })
        .detach();
    }

    /// Takes the Keeper's answer to an attach. A refusal, as for a terminal whose program
    /// ended since it was listed, adds nothing: the shared list drops that terminal when
    /// it is told of its end, and it is asked for again only if the list changes.
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
        // The user is not looking at this terminal: it takes the focus only from the
        // pane's own handle, which no terminal has, so that the keys of a Window that
        // opens on its terminals go to the one it shows.
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

    /// What one of the terminal views tells, a view of a terminal of `workspace`.
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

    /// What the Keeper tells that the pane shows: a program that ended. A program that ends
    /// between the Keeper's answer to an attach and the pane taking it is remembered for
    /// the view the pane makes then.
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
        if let Some(layout) = self.layout_mut()
            && n < layout.shown.len()
            && layout.focus_slot != n
        {
            layout.focus_slot = n;
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

    /// The terminal `by` places after the focused one among those of the workspace on
    /// screen, round at the ends.
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
        let (warning, warning_text) = (theme.warning, theme.warning_foreground);
        let shown = self.shown();
        // The launchers and the resume controls take a row of their own at the top
        // (`launchers.rs`), so that no terminal's first rows lie under them; the room the
        // canvas measures is what lies below them.
        let launchers = self.render_launchers(cx);
        // The question an Adapter asks before it writes into its CLI's settings lies in
        // the same top row, under them (`consent.rs`).
        let consent = self.render_consent(cx);
        let slots = shown.iter().enumerate().filter_map(|(n, id)| {
            // The name comes from the entry the view is taken from, so that no view is
            // drawn under the name of another terminal.
            let entry = self.entry(id)?;
            let view = entry.view.clone();
            let name = entry.id.as_str().to_owned();
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
            // The mark of an agent that waits for the user sits over the top right corner
            // of its terminal, and takes no mouse event from it.
            let mark = self.waiting(id, cx).then(|| {
                div()
                    .debug_selector(move || format!("terminal-waiting-{n}"))
                    .absolute()
                    .top_2()
                    .right_3()
                    .px_2()
                    .py_0p5()
                    .rounded_md()
                    .text_xs()
                    .bg(warning)
                    .text_color(warning_text)
                    .child("Waiting for permission")
            });
            // Over the line that says older output is gone, a rule under its text.
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
                        .border_color(border)
                });
            // The divider is a sibling of the slot, so the slot's bounds are exactly its
            // view's.
            let divider = (n > 0).then(|| div().flex_none().w_px().h_full().bg(border));
            // The view sits in an element of its own that names its terminal, so that
            // what is drawn can be told from what the pane says is shown.
            let named = div()
                .debug_selector(move || format!("terminal-{name}"))
                .size_full()
                .child(view);
            // The review attached to the agent's next message, with its controls, over the
            // bottom of its terminal.
            let attached = self.render_attachment(n, id, cx);
            Some((
                divider,
                slot.relative()
                    .child(named)
                    .children(cut)
                    .children(mark)
                    .children(attached),
            ))
        });
        let slots = slots.flat_map(|(divider, slot)| divider.into_iter().chain([slot]));
        let empty = shown.is_empty().then(|| {
            div()
                .debug_selector(|| "terminal-how-to-open".to_owned())
                .p_3()
                .text_sm()
                .text_color(muted)
                .child(how_to_open())
        });
        // The room the pane has, as columns and rows of the cell a view draws, which a
        // terminal attached before any is laid out is sized from.
        let cell = TerminalView::metrics(window, cx).2;
        let (weak, seen) = (cx.weak_entity(), self.room_seen.clone());
        let room = canvas(
            |bounds, _, _| bounds,
            move |_, bounds, window, cx| {
                let columns = (f32::from(bounds.size.width) / f32::from(cell.width)).floor() as u16;
                let rows = (f32::from(bounds.size.height) / f32::from(cell.height)).floor() as u16;
                let room = TerminalSize { columns, rows };
                if columns == 0 || rows == 0 || seen.get() == Some(room) {
                    return;
                }
                seen.set(Some(room));
                let weak = weak.clone();
                window.defer(cx, move |_, cx| {
                    weak.update(cx, |pane, cx| pane.room_found(room, cx)).ok();
                });
            },
        )
        .absolute()
        .size_full();
        let notice = self.notice.clone().map(|notice| {
            div()
                .flex_none()
                .px_3()
                .py_1()
                .text_sm()
                .text_color(cx.theme().danger)
                .child(notice)
        });
        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .size_full()
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
                    .child(div().flex_1().min_h_0().flex().flex_row().children(slots)),
            )
    }
}

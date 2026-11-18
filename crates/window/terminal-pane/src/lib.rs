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

mod emulator;
mod keys;
mod mouse;
mod view;

use std::time::Duration;

use gpui_kit::*;
use yaar_frame::{AskError, Asks, BuildPane, Control, Models, Pane, PaneEntry, Place, Sends};
use yaar_link::{Program, Reply, Request, RequestKind, TerminalId, TerminalSize};

pub use emulator::{Cell, CellPlace, Color, Cursor, Emulator, HISTORY_LINES, Screen, Style};
pub use keys::{
    IMPLEMENTED_FLAGS, KeyboardFlags, Modes, OptionKey, encode_focus, encode_key, encode_paste,
    encode_release,
};
pub use mouse::{MouseAction, MouseButton, MouseMode, encode_mouse};
pub use view::TerminalView;

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
/// [`NewTerminal`] works anywhere, on `secondary-t`, and sends
/// [`RequestKind::OpenTerminal`]. The table of the other actions is the builder's: see
/// the slice's plan.
pub fn controls() -> Vec<Control> {
    vec![Control {
        title: "New Terminal",
        action: || Box::new(NewTerminal),
        keystroke: "secondary-t",
        place: Place::Anywhere,
        sends: Some(Sends::Request(RequestKind::OpenTerminal)),
        asks: Asks::Nothing,
    }]
}

/// The Terminal pane: the terminals the Window opened, the one or two it shows, and the
/// one that has the focus.
///
/// It takes [`NewTerminal`] from wherever the focus is in the window, through the Frame's
/// routes.
pub struct TerminalPane {
    models: Models,
    focus: FocusHandle,
    /// The size the pane last laid a terminal out at, which a new terminal is opened at.
    size: TerminalSize,
    /// Why the last terminal asked for was not opened, until the next is asked for.
    notice: Option<String>,
    /// The question for a new terminal, while it waits for its answer.
    opening: Option<Task<()>>,
}

impl TerminalPane {
    /// An empty pane of the window whose models are `models`.
    pub fn new(models: &Models, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let _ = window;
        models.routes.route(&cx.entity(), Self::new_terminal);
        Self {
            models: models.clone(),
            focus: cx.focus_handle(),
            size: FIRST_SIZE,
            notice: None,
            opening: None,
        }
    }

    /// The terminals of the pane, in the order they were opened, those whose program
    /// ended included.
    pub fn terminals(&self) -> Vec<TerminalId> {
        todo!()
    }

    /// The view of one terminal of the pane.
    pub fn terminal(&self, id: &TerminalId) -> Option<Entity<TerminalView>> {
        let _ = id;
        todo!()
    }

    /// The terminals drawn now, left to right: none, one, or two in the split view.
    pub fn shown(&self) -> Vec<TerminalId> {
        todo!()
    }

    /// Whether the split view is on.
    pub fn split(&self) -> bool {
        todo!()
    }

    /// The terminal the keys go to while the pane has the focus: one of those shown.
    pub fn focused(&self) -> Option<TerminalId> {
        todo!()
    }

    /// Why the last terminal asked for was not opened; none once another is asked for.
    /// The pane draws this very sentence.
    pub fn notice(&self) -> Option<String> {
        self.notice.clone()
    }

    /// Asks the Keeper for a new terminal for the workspace on screen, at the size the
    /// pane lays terminals out at. With no workspace on screen there is nothing to open
    /// one in, and nothing is asked.
    fn new_terminal(&mut self, _: &NewTerminal, _: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace) = self.models.workspaces.read(cx).selected().cloned() else {
            return;
        };
        self.notice = None;
        let request = Request::OpenTerminal {
            workspace,
            program: Program::Shell,
            size: self.size,
        };
        let asked = self
            .models
            .link
            .update(cx, |link, cx| link.ask(request, cx));
        self.opening = Some(cx.spawn(async move |pane, cx| {
            let answer = asked.await;
            pane.update(cx, |pane, cx| pane.opened(answer, cx)).ok();
        }));
        cx.notify();
    }

    /// Takes the Keeper's answer to the question for a new terminal.
    fn opened(&mut self, answer: Result<Reply, AskError>, cx: &mut Context<Self>) {
        self.opening = None;
        match answer {
            Ok(Reply::TerminalOpened { terminal }) => {
                let _ = terminal;
                todo!()
            }
            Ok(_) => self.notice = Some(AskError::Unexpected.to_string()),
            Err(error) => self.notice = Some(error.to_string()),
        }
        cx.notify();
    }
}

impl Render for TerminalPane {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .children(self.notice.clone())
    }
}

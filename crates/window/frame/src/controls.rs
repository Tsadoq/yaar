//! The actions of the Window that have a key, in one table.
//!
//! Key bindings are made from the table, and so will menus and the list of keys a user
//! can read. A crate that brings actions of its own, as a pane does, has a function that
//! returns its part of the table; the Window program puts the parts together.
//!
//! Every message of the Link that a user starts has a row here or in the table of a pane:
//! [`Control::sends`] names it. A row that names none sends none.
//!
//! The keys of the Frame and of the Window program are written `secondary-`, GPUI's name
//! for the modifier that is Cmd on macOS and Ctrl on every other system, and work
//! [`Place::Anywhere`]. A Cmd key is bound with no context: it works wherever the focus
//! is, a terminal included. Any other key is bound with a context, so that it cannot fire
//! where a program expects it: a Ctrl key of the Frame or of the Window program with
//! [`KEY_CONTEXT`], which the root element of the Frame sets, and a key of a pane with
//! the context of that pane. A row of a table says where its key works, and
//! [`Control::context`] alone says which context that is on the system the app is built
//! for.
//!
//! A program in a terminal takes every key but a few. The element that shows such a
//! program sets [`PROGRAM_CONTEXT`]. There a key of the Frame or of the Window program
//! works only where it is a Cmd key, on macOS; every other key reaches the program, except
//! the documented kept keys: a row with [`Place::Kept`], and a row bound in
//! [`PROGRAM_CONTEXT`] itself, as a terminal's own copy and paste.
//!
//! The toolkit binds a few keys of its own for the whole window: Tab and Shift-Tab, which
//! move the focus between fields, and, off macOS, Ctrl-C and Ctrl-Shift-I. A program needs
//! them, and [`release_program_keys`] gives them back to it.

use gpui_kit::*;
use yaar_link::RequestKind;

actions!(
    yaar,
    [
        /// Asks the user for a folder and opens it as a workspace.
        OpenFolder,
        /// Asks the user for a branch or a commit and makes it the base of the workspace
        /// on screen.
        SetBase,
        /// Pins the sidebar in the form it does not have now: a rail if it is the full
        /// list, the full list if it is a rail.
        ToggleSidebar,
        /// Unpins the sidebar: its form follows the width of the window again.
        UnpinSidebar,
        /// Puts the next workspace of the sidebar on screen.
        SelectNextWorkspace,
        /// Puts the previous workspace of the sidebar on screen.
        SelectPreviousWorkspace,
        /// Moves the focus to the next pane, in the order the panes are laid out, and from
        /// the last to the first.
        FocusNextPane,
        /// Moves the focus to the previous pane, and from the first to the last.
        FocusPreviousPane,
        /// Moves the panes after the first to the next place beside the first: from the
        /// right to below it, from below to its left, from its left to the right. The
        /// place is a setting of the app: the action sets the next one, which the Keeper
        /// keeps. See [`crate::Beside`].
        MovePanes,
        /// Quits the Window, the plain way: the Keeper, the agents and every program of
        /// the terminals go on, and a Window opened again shows them as they are. The
        /// Window program says what quitting does; the Frame sends nothing for it.
        Quit,
        /// Quits the Window and asks the Keeper to stop everything: every program of the
        /// terminals and what they started, the agents among them, and then itself. It
        /// sends the stop-everything frame, `yaar_link::Message::Shutdown`.
        QuitAndStopAgents,
        /// Puts on screen the workspace of the notice of a turn that ended where the
        /// Window did not show it, and takes the notice away.
        ShowFinishedTurn,
        /// Opens the settings screen in the place of the panes and gives it the focus;
        /// closes it, and gives the focus back, when it is open. The Frame handles it. See
        /// `crate::settings_screen`.
        OpenSettings,
        /// Closes the settings screen and gives the focus back to where it was when the
        /// screen opened. The Frame handles it, and the screen's own table binds its key.
        CloseSettings,
        /// Moves the cursor of the settings screen to the next row.
        NextSetting,
        /// Moves it to the previous row.
        PreviousSetting,
        /// Chooses the next value of the row under the cursor of the settings screen.
        NextValue,
        /// Chooses the previous value of the row under the cursor.
        PreviousValue,
        /// Acts on the row under the cursor, where the row acts: the base opens its field.
        OpenSetting,
        /// Deletes at once the recorded states nothing refers to, with what only they held.
        CleanUpStates,
        /// Asks the user for the name of a new branch in a one-line field, and makes a
        /// worktree on it, from the branch of the workspace on screen, with the chosen agent
        /// CLI started in it. The Frame handles it: Enter in the field wants the worktree
        /// ([`crate::Wanted::Worktree`]), which the terminal pane sends as
        /// `CreateWorktree`.
        NewWorktree,
        /// Closes the window: on macOS the app's window; the app quits the plain way when its
        /// last window is closed.
        CloseWindow,
        /// Hides the app, on macOS.
        HideApp,
        /// Hides every other app, on macOS.
        HideOtherApps,
        /// Puts the window in the Dock, on macOS.
        Minimize,
    ]
);

/// The key context of the Frame. Its root element sets it, so it holds wherever the focus
/// is in what the Frame draws, a pane included.
pub const KEY_CONTEXT: &str = "Frame";

/// The key context of the settings screen while it has the focus.
pub use crate::settings_screen::SETTINGS_CONTEXT;

/// The key context of an element that hands every key to a program, as a terminal does.
/// Inside it only the kept keys run actions of the app: see the doc of this module.
pub const PROGRAM_CONTEXT: &str = "Program";

/// Whether `secondary`, the modifier of the app's own keys, is the Cmd key on the system
/// the app is built for. It is on macOS, and it is Ctrl on every other system.
const SECONDARY_IS_CMD: bool = cfg!(target_os = "macos");

/// Where the key of a control works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Wherever the focus is in the window, except, on a system where `secondary` is not
    /// the Cmd key, inside an element that sets [`PROGRAM_CONTEXT`]: there the key goes
    /// to the program. A key of the Frame, of the Window program, or a pane's key that
    /// works from anywhere.
    Anywhere,
    /// Where this GPUI key context is, and nowhere else: a key of a pane.
    In(&'static str),
    /// Wherever the focus is in the window, inside an element that sets
    /// [`PROGRAM_CONTEXT`] too, on every system: one of the documented kept keys of the
    /// Frame, which never reach a program. The only place of a key of the Frame that works
    /// inside a program off macOS. It is bound with [`KEY_CONTEXT`] on every system, since
    /// it is no Cmd key.
    Kept,
}

impl Place {
    /// The context a key that works here is bound with, on a system where `secondary`
    /// is the Cmd key and on one where it is not. Only a Cmd key may be bound with none.
    /// This is the one place where the system decides anything about a key of the app.
    fn context(self, secondary_is_cmd: bool) -> Option<&'static str> {
        match self {
            Place::In(context) => Some(context),
            Place::Kept => Some(KEY_CONTEXT),
            Place::Anywhere if secondary_is_cmd => None,
            // The Frame's context holds in a program too, so the key is also asked not to
            // be inside one: GPUI's `!` looks at every context from the root to the focus.
            Place::Anywhere => Some("Frame && !Program"),
        }
    }
}

/// Writes `secondary` out in each keystroke of `key`, as `cmd` when `secondary_is_cmd` and
/// as `ctrl` when not. Only a modifier, which comes before the last part of a keystroke,
/// is read: the last part is the key itself.
fn write_key(key: &str, secondary_is_cmd: bool) -> String {
    let modifier = if secondary_is_cmd { "cmd" } else { "ctrl" };
    key.split(' ')
        .map(|keystroke| {
            let mut written = String::new();
            let mut rest = keystroke;
            // A `-` that ends the keystroke is the key, as in `secondary--`.
            while let Some((part, tail)) = rest.split_once('-')
                && !tail.is_empty()
            {
                written.push_str(if part == "secondary" { modifier } else { part });
                written.push('-');
                rest = tail;
            }
            written.push_str(rest);
            written
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What the Window asks the user for after an action, before it has anything to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asks {
    /// Nothing: the action does its work at once.
    Nothing,
    /// A folder, in the system's dialog for choosing one.
    Folder,
    /// One line of text, in a field of the Window: the keys typed next go to the field,
    /// Enter takes the text, Escape gives up.
    Text,
}

/// A message of the Link that a user starts, and an action therefore ends in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sends {
    /// A request of this kind, whose origin is [`yaar_link::Origin::User`].
    Request(RequestKind),
    /// The stop-everything frame, [`yaar_link::Message::Shutdown`], which is not a
    /// request and so has no kind.
    StopEverything,
}

/// One action of the Window with its key.
#[derive(Debug, Clone, Copy)]
pub struct Control {
    /// What the action is called where a user reads it.
    pub title: &'static str,
    /// Makes the action.
    pub action: fn() -> Box<dyn Action>,
    /// The key that runs it, as GPUI writes keys. `secondary` is Cmd on macOS and Ctrl
    /// on every other system.
    pub keystroke: &'static str,
    /// Where the key works. A key that works anywhere is written with `secondary`.
    pub place: Place,
    /// The message of the Link that the action ends in. None for an action that sends
    /// nothing a user starts; what the Window then asks by itself, to have something to
    /// show, is not the user's.
    pub sends: Option<Sends>,
    /// What the user is asked for on the way.
    pub asks: Asks,
}

impl Control {
    /// The GPUI key context the key is bound with on the system the app is built for;
    /// none means that it is bound for the whole app. On macOS a key that works
    /// [`Place::Anywhere`] has none. On every other system it has [`KEY_CONTEXT`] and not
    /// [`PROGRAM_CONTEXT`], written `Frame && !Program`, and no key is without a context.
    pub fn context(&self) -> Option<&'static str> {
        self.place.context(SECONDARY_IS_CMD)
    }

    /// The key as [`bind`] binds it on the system the app is built for: the keystroke
    /// with `secondary` written out, as `cmd` where it is the Cmd key and as `ctrl`
    /// elsewhere; every other key as the table writes it. `secondary-alt-p` is
    /// `cmd-alt-p` on macOS and `ctrl-alt-p` on Linux.
    pub fn key(&self) -> String {
        write_key(self.keystroke, SECONDARY_IS_CMD)
    }
}

/// The keys the toolkit binds for the whole window that a program in a terminal must
/// get: Tab and Shift-Tab, which the toolkit's root binds to move the focus, and off
/// macOS Ctrl-C, which it binds to copy, and Ctrl-Shift-I, which opens the inspector in a
/// build with debug assertions. On macOS the toolkit's copy and inspector are Cmd keys,
/// which are the app's.
pub const PROGRAM_KEYS: &[&str] = if SECONDARY_IS_CMD {
    &["tab", "shift-tab"]
} else {
    &["tab", "shift-tab", "ctrl-c", "ctrl-shift-i"]
};

/// Gives a program the keys of [`PROGRAM_KEYS`] back: binds each to nothing in
/// [`PROGRAM_CONTEXT`]. A binding to nothing in a deeper context hides the bindings of the
/// same key that it outranks, so the key goes on, as no action, to the element that shows
/// the program, whose key listener sends it.
///
/// Call it after `gpui_kit::init`: of two bindings at the same depth the later wins, and
/// the toolkit binds the inspector's key with no context, which is as deep as any.
pub fn release_program_keys(cx: &mut App) {
    cx.bind_keys(
        PROGRAM_KEYS
            .iter()
            .map(|key| KeyBinding::new(key, NoAction, Some(PROGRAM_CONTEXT))),
    );
}

/// The actions of the Frame itself.
pub fn controls() -> Vec<Control> {
    vec![
        Control {
            title: "Open Folder…",
            action: || Box::new(OpenFolder),
            keystroke: "secondary-o",
            place: Place::Anywhere,
            sends: Some(Sends::Request(RequestKind::OpenFolder)),
            asks: Asks::Folder,
        },
        Control {
            title: "Set Base…",
            action: || Box::new(SetBase),
            keystroke: "secondary-shift-b",
            place: Place::Anywhere,
            sends: Some(Sends::Request(RequestKind::SetBase)),
            asks: Asks::Text,
        },
        Control {
            title: "Settings…",
            action: || Box::new(OpenSettings),
            keystroke: "secondary-,",
            place: Place::Anywhere,
            sends: None,
            asks: Asks::Nothing,
        },
        Control {
            title: "New Worktree with Agent…",
            action: || Box::new(NewWorktree),
            keystroke: "secondary-shift-n",
            place: Place::Anywhere,
            sends: Some(Sends::Request(RequestKind::CreateWorktree)),
            asks: Asks::Text,
        },
        Control {
            title: "Toggle Sidebar",
            action: || Box::new(ToggleSidebar),
            keystroke: "secondary-b",
            place: Place::Anywhere,
            sends: None,
            asks: Asks::Nothing,
        },
        Control {
            title: "Sidebar by Window Width",
            action: || Box::new(UnpinSidebar),
            keystroke: "secondary-alt-b",
            place: Place::Anywhere,
            sends: None,
            asks: Asks::Nothing,
        },
        Control {
            title: "Next Workspace",
            action: || Box::new(SelectNextWorkspace),
            keystroke: "secondary-alt-down",
            place: Place::Anywhere,
            sends: None,
            asks: Asks::Nothing,
        },
        Control {
            title: "Previous Workspace",
            action: || Box::new(SelectPreviousWorkspace),
            keystroke: "secondary-alt-up",
            place: Place::Anywhere,
            sends: None,
            asks: Asks::Nothing,
        },
        Control {
            title: "Next Pane",
            action: || Box::new(FocusNextPane),
            keystroke: "ctrl-tab",
            place: Place::Kept,
            sends: None,
            asks: Asks::Nothing,
        },
        Control {
            title: "Previous Pane",
            action: || Box::new(FocusPreviousPane),
            keystroke: "ctrl-shift-tab",
            place: Place::Kept,
            sends: None,
            asks: Asks::Nothing,
        },
        Control {
            title: "Move Panes",
            action: || Box::new(MovePanes),
            keystroke: "secondary-alt-p",
            place: Place::Anywhere,
            // The place of the panes is a setting the Keeper keeps.
            sends: Some(Sends::Request(RequestKind::SetSetting)),
            asks: Asks::Nothing,
        },
        Control {
            title: "Show Finished Turn",
            action: || Box::new(ShowFinishedTurn),
            keystroke: "secondary-alt-n",
            place: Place::Anywhere,
            sends: None,
            asks: Asks::Nothing,
        },
        Control {
            title: "Quit",
            action: || Box::new(Quit),
            keystroke: "secondary-q",
            place: Place::Anywhere,
            sends: None,
            asks: Asks::Nothing,
        },
        Control {
            title: "Quit and Stop Agents",
            action: || Box::new(QuitAndStopAgents),
            keystroke: "secondary-alt-q",
            place: Place::Anywhere,
            sends: Some(Sends::StopEverything),
            asks: Asks::Nothing,
        },
    ]
}

/// Binds the key of each control, as [`Control::key`] writes it, to its action, in the
/// context that [`Control::context`] gives: for the whole app where that is none.
///
/// The tables are written by hand and never change while the app runs, so a key, an
/// empty one included, or a context that cannot be read is a mistake in the code: it
/// panics, naming the control.
pub fn bind(controls: &[Control], cx: &mut App) {
    let bindings: Vec<KeyBinding> = controls
        .iter()
        .map(|control| {
            let context = control.context().map(|context| {
                KeyBindingContextPredicate::parse(context)
                    .unwrap_or_else(|error| {
                        panic!(
                            "the context {context:?} of {:?} is not a key context: {error}",
                            control.title
                        )
                    })
                    .into()
            });
            // The parser reads no keystroke at all as no binding, and binds nothing.
            assert!(
                !control.keystroke.trim().is_empty(),
                "the key {:?} of {:?} is not a key: it is empty",
                control.keystroke,
                control.title
            );
            KeyBinding::load(&control.key(), (control.action)(), context, false, None, cx)
                .unwrap_or_else(|error| {
                    panic!(
                        "the key {:?} of {:?} is not a key: {error}",
                        control.keystroke, control.title
                    )
                })
        })
        .collect();
    cx.bind_keys(bindings);
}

#[cfg(test)]
mod tests {
    use super::{KEY_CONTEXT, PROGRAM_CONTEXT, Place, write_key};

    #[test]
    fn a_key_that_works_anywhere_has_no_context_where_it_is_a_cmd_key_and_the_frames_outside_a_program_elsewhere()
     {
        assert_eq!(Place::Anywhere.context(true), None);
        assert_eq!(
            Place::Anywhere.context(false),
            Some(format!("{KEY_CONTEXT} && !{PROGRAM_CONTEXT}").as_str())
        );
    }

    #[test]
    fn a_kept_key_has_the_context_of_the_frame_on_every_system() {
        for secondary_is_cmd in [true, false] {
            assert_eq!(Place::Kept.context(secondary_is_cmd), Some(KEY_CONTEXT));
        }
    }

    #[test]
    fn a_key_of_a_pane_has_the_context_of_the_pane_on_every_system() {
        for secondary_is_cmd in [true, false] {
            assert_eq!(Place::In("Pane").context(secondary_is_cmd), Some("Pane"));
        }
    }

    #[test]
    fn secondary_is_written_out_as_cmd_or_as_ctrl() {
        assert_eq!(write_key("secondary-alt-p", true), "cmd-alt-p");
        assert_eq!(write_key("secondary-alt-p", false), "ctrl-alt-p");
        assert_eq!(write_key("secondary-o", true), "cmd-o");
        assert_eq!(write_key("secondary-shift-b", false), "ctrl-shift-b");
    }

    #[test]
    fn a_key_with_no_secondary_stays_as_written() {
        for secondary_is_cmd in [true, false] {
            assert_eq!(write_key("ctrl-tab", secondary_is_cmd), "ctrl-tab");
            assert_eq!(
                write_key("ctrl-shift-tab", secondary_is_cmd),
                "ctrl-shift-tab"
            );
            assert_eq!(write_key("a", secondary_is_cmd), "a");
        }
    }

    #[test]
    fn every_keystroke_of_a_key_with_several_is_written_out() {
        assert_eq!(
            write_key("secondary-k secondary-shift-c", false),
            "ctrl-k ctrl-shift-c"
        );
        assert_eq!(write_key("secondary-k c", true), "cmd-k c");
    }

    #[test]
    fn the_word_secondary_is_taken_for_the_modifier_only_where_it_is_one() {
        // A key part is the last of its keystroke: it is never a modifier.
        assert_eq!(write_key("secondary--", true), "cmd--");
        assert_eq!(write_key("secondary", true), "secondary");
        assert_eq!(write_key("ctrl-secondary", false), "ctrl-secondary");
    }
}

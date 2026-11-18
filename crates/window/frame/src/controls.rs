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
    ]
);

/// The key context of the Frame. Its root element sets it, so it holds wherever the focus
/// is in what the Frame draws, a pane included.
pub const KEY_CONTEXT: &str = "Frame";

/// Whether `secondary`, the modifier of the app's own keys, is the Cmd key on the system
/// the app is built for. It is on macOS, and it is Ctrl on every other system.
const SECONDARY_IS_CMD: bool = cfg!(target_os = "macos");

/// Where the key of a control works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Wherever the focus is in the window: a key of the Frame or of the Window program.
    Anywhere,
    /// Where this GPUI key context is, and nowhere else: a key of a pane.
    In(&'static str),
}

impl Place {
    /// The context a key that works here is bound with, on a system where `secondary`
    /// is the Cmd key and on one where it is not. Only a Cmd key may be bound with none.
    /// This is the one place where the system decides anything about a key of the app.
    fn context(self, secondary_is_cmd: bool) -> Option<&'static str> {
        match self {
            Place::In(context) => Some(context),
            Place::Anywhere if secondary_is_cmd => None,
            Place::Anywhere => Some(KEY_CONTEXT),
        }
    }
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
    /// [`Place::Anywhere`] has none. On every other system it has [`KEY_CONTEXT`], and
    /// no key is without a context.
    pub fn context(&self) -> Option<&'static str> {
        self.place.context(SECONDARY_IS_CMD)
    }
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
    ]
}

/// Binds the key of each control to its action, in the context that
/// [`Control::context`] gives: for the whole app where that is none.
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
            KeyBinding::load(
                control.keystroke,
                (control.action)(),
                context,
                false,
                None,
                cx,
            )
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
    use super::{KEY_CONTEXT, Place};

    #[test]
    fn a_key_that_works_anywhere_has_no_context_where_it_is_a_cmd_key_and_the_frames_elsewhere() {
        assert_eq!(Place::Anywhere.context(true), None);
        assert_eq!(Place::Anywhere.context(false), Some(KEY_CONTEXT));
    }

    #[test]
    fn a_key_of_a_pane_has_the_context_of_the_pane_on_every_system() {
        for secondary_is_cmd in [true, false] {
            assert_eq!(Place::In("Pane").context(secondary_is_cmd), Some("Pane"));
        }
    }
}

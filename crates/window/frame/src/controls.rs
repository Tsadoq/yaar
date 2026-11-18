//! The actions of the Window that have a key, in one table.
//!
//! Key bindings are made from the table, and so will menus and the list of keys a user
//! can read. A crate that brings actions of its own, as a pane does, has a function that
//! returns its part of the table; the Window program puts the parts together.
//!
//! Every message of the Link that a user starts has a row here or in the table of a pane:
//! [`Control::sends`] names it. A row that names none sends none.
//!
//! The keys of the Frame and of the Window program are Cmd keys, written `cmd-`: Cmd on
//! macOS, the Super key elsewhere. A Cmd key works wherever the focus is, a terminal
//! included, and needs no context. Any other key is bound with a context, so that it
//! cannot fire where a program expects it.

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
    /// The key that runs it, as GPUI writes keys. `cmd` is Cmd on macOS and the Super
    /// key elsewhere.
    pub keystroke: &'static str,
    /// Where the key works, as a GPUI key context; none means anywhere in the window,
    /// and only a Cmd key may have none.
    pub context: Option<&'static str>,
    /// The message of the Link that the action ends in. None for an action that sends
    /// nothing a user starts; what the Window then asks by itself, to have something to
    /// show, is not the user's.
    pub sends: Option<Sends>,
    /// What the user is asked for on the way.
    pub asks: Asks,
}

/// The actions of the Frame itself.
pub fn controls() -> Vec<Control> {
    vec![
        Control {
            title: "Open Folder…",
            action: || Box::new(OpenFolder),
            keystroke: "cmd-o",
            context: None,
            sends: Some(Sends::Request(RequestKind::OpenFolder)),
            asks: Asks::Folder,
        },
        Control {
            title: "Set Base…",
            action: || Box::new(SetBase),
            keystroke: "cmd-shift-b",
            context: None,
            sends: Some(Sends::Request(RequestKind::SetBase)),
            asks: Asks::Text,
        },
        Control {
            title: "Toggle Sidebar",
            action: || Box::new(ToggleSidebar),
            keystroke: "cmd-b",
            context: None,
            sends: None,
            asks: Asks::Nothing,
        },
        Control {
            title: "Sidebar by Window Width",
            action: || Box::new(UnpinSidebar),
            keystroke: "cmd-alt-b",
            context: None,
            sends: None,
            asks: Asks::Nothing,
        },
        Control {
            title: "Next Workspace",
            action: || Box::new(SelectNextWorkspace),
            keystroke: "cmd-alt-down",
            context: None,
            sends: None,
            asks: Asks::Nothing,
        },
        Control {
            title: "Previous Workspace",
            action: || Box::new(SelectPreviousWorkspace),
            keystroke: "cmd-alt-up",
            context: None,
            sends: None,
            asks: Asks::Nothing,
        },
    ]
}

/// Binds the key of each control to its action, for the whole app.
///
/// The tables are written by hand and never change while the app runs, so a key or a
/// context that cannot be read is a mistake in the code: it panics, naming the control.
pub fn bind(controls: &[Control], cx: &mut App) {
    let bindings: Vec<KeyBinding> = controls
        .iter()
        .map(|control| {
            let context = control.context.map(|context| {
                KeyBindingContextPredicate::parse(context)
                    .unwrap_or_else(|error| {
                        panic!(
                            "the context {context:?} of {:?} is not a key context: {error}",
                            control.title
                        )
                    })
                    .into()
            });
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

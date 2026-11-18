//! The actions of the Window that have a key, in one table.
//!
//! Key bindings are made from the table, and so will menus and the list of keys a user
//! can read. A crate that brings actions of its own, as a pane does, has a function that
//! returns its part of the table; the Window program puts the parts together.
//!
//! Every request of the Link that a user starts has a row here or in the table of a pane:
//! [`Control::sends`] names it.

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

/// One action of the Window with its key.
#[derive(Debug, Clone, Copy)]
pub struct Control {
    /// What the action is called where a user reads it.
    pub title: &'static str,
    /// Makes the action.
    pub action: fn() -> Box<dyn Action>,
    /// The key that runs it, as GPUI writes keys. `secondary` is Cmd on macOS and Ctrl
    /// elsewhere.
    pub keystroke: &'static str,
    /// Where the key works, as a GPUI key context; none means anywhere in the window.
    pub context: Option<&'static str>,
    /// The request of the Link that the action ends in, if it ends in one.
    pub sends: Option<RequestKind>,
    /// What the user is asked for on the way.
    pub asks: Asks,
}

/// The actions of the Frame itself.
pub fn controls() -> Vec<Control> {
    vec![
        Control {
            title: "Open Folder…",
            action: || Box::new(OpenFolder),
            keystroke: "secondary-o",
            context: None,
            sends: Some(RequestKind::OpenFolder),
            asks: Asks::Folder,
        },
        Control {
            title: "Set Base…",
            action: || Box::new(SetBase),
            keystroke: "secondary-shift-b",
            context: None,
            sends: Some(RequestKind::SetBase),
            asks: Asks::Text,
        },
        Control {
            title: "Toggle Sidebar",
            action: || Box::new(ToggleSidebar),
            keystroke: "secondary-b",
            context: None,
            sends: None,
            asks: Asks::Nothing,
        },
        Control {
            title: "Sidebar by Window Width",
            action: || Box::new(UnpinSidebar),
            keystroke: "secondary-alt-b",
            context: None,
            sends: None,
            asks: Asks::Nothing,
        },
        Control {
            title: "Next Workspace",
            action: || Box::new(SelectNextWorkspace),
            keystroke: "secondary-alt-down",
            context: None,
            sends: None,
            asks: Asks::Nothing,
        },
        Control {
            title: "Previous Workspace",
            action: || Box::new(SelectPreviousWorkspace),
            keystroke: "secondary-alt-up",
            context: None,
            sends: None,
            asks: Asks::Nothing,
        },
    ]
}

/// Binds the key of each control to its action, for the whole app.
pub fn bind(controls: &[Control], cx: &mut App) {
    todo!()
}

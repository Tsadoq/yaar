//! What a click, a drag or the wheel sends to the program of a terminal.
//!
//! A program that asked for mouse reports gets them in the form it asked for, SGR or the
//! old one, for the actions its mode names. A program that asked for none gets nothing:
//! a drag then selects text in the pane, and the wheel scrolls the history, or sends the
//! arrow keys on the alternate screen when the program asked for that.

use gpui_kit::Modifiers;

use crate::keys::Modes;

/// Which mouse actions a program asked to be told of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseMode {
    /// None.
    #[default]
    Off,
    /// Presses and releases, and the wheel (mode 1000).
    Click,
    /// Those, and moves while a button is held (mode 1002).
    Drag,
    /// Those, and every move (mode 1003).
    Motion,
}

/// A button of the mouse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    /// The left one.
    Left,
    /// The middle one, or the wheel pressed.
    Middle,
    /// The right one.
    Right,
}

/// One action of the mouse over a cell of the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    /// A button went down.
    Press(MouseButton),
    /// A button came up.
    Release(MouseButton),
    /// The pointer moved, with this button held or none.
    Move(Option<MouseButton>),
    /// The wheel turned one step up.
    WheelUp,
    /// The wheel turned one step down.
    WheelDown,
}

/// The bytes a mouse action over the cell at `column` and `line`, both counted from 0,
/// sends to the program; none when the program did not ask to be told of it.
pub fn encode_mouse(
    action: MouseAction,
    column: u16,
    line: u16,
    modifiers: &Modifiers,
    modes: &Modes,
) -> Option<Vec<u8>> {
    let _ = (action, column, line, modifiers, modes);
    todo!()
}

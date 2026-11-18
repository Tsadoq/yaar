//! What a key, a paste or a change of focus sends to the program of a terminal.
//!
//! The encoder follows the terminal's state, as [`crate::Modes`] reads it from the
//! emulator: xterm's forms where the program asked for nothing, the application cursor
//! keys where it asked for them, and the kitty keyboard protocol where it pushed its
//! flags. Of those flags it implements 1, every key with Ctrl or Option and Escape in the
//! protocol's own form, and 2, the repeat and the release of a key; see
//! [`crate::IMPLEMENTED_FLAGS`].
//!
//! A key that types text goes to the program through the view's input handler, which
//! also composes dead keys and the text of an input method: [`encode_key`] gives none for
//! it. A Cmd key never reaches a program: it gives none either.

use gpui_kit::Keystroke;

use crate::modes::Modes;

/// What Option does, on macOS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionKey {
    /// It is Meta: Option with a key sends that key with the Meta prefix, or in the kitty
    /// form with the Alt modifier.
    Meta,
    /// It types the character the keyboard layout gives, as macOS does everywhere else.
    Characters,
}

impl OptionKey {
    /// What Option does until the user's setting says otherwise: on macOS, Meta when
    /// `layout_id` is exactly `com.apple.keylayout.US`, and characters on every other
    /// layout, U.S. Extended and U.S. International among them; Meta on every system but
    /// macOS (`mac` false), where Alt is always Meta.
    pub fn for_layout(layout_id: &str, mac: bool) -> OptionKey {
        let _ = (layout_id, mac);
        todo!()
    }
}

/// The bytes a key press sends to the program, or none when the key is not the
/// encoder's: text the input handler sends, a Cmd key, or a key that sends nothing.
/// `repeat` says that the key is held and this press is a repeat of the one before.
pub fn encode_key(
    keystroke: &Keystroke,
    modes: &Modes,
    option: OptionKey,
    repeat: bool,
) -> Option<Vec<u8>> {
    let _ = (keystroke, modes, option, repeat);
    todo!()
}

/// The bytes the release of a key sends: only under flag 2, and only for a key whose
/// press the encoder sent.
pub fn encode_release(keystroke: &Keystroke, modes: &Modes, option: OptionKey) -> Option<Vec<u8>> {
    let _ = (keystroke, modes, option);
    todo!()
}

/// The bytes of pasted text: the text, with line ends as Return sends them, and between
/// the two marks of bracketed paste when the program asked for it. Text that holds the
/// closing mark loses it, so that a paste cannot end itself early.
pub fn encode_paste(text: &str, modes: &Modes) -> Vec<u8> {
    let _ = (text, modes);
    todo!()
}

/// The bytes that tell the program the terminal gained or lost the focus, when it asked
/// for them.
pub fn encode_focus(focused: bool, modes: &Modes) -> Option<Vec<u8>> {
    let _ = (focused, modes);
    todo!()
}

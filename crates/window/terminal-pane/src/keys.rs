//! What a key, a paste or a change of focus sends to the program of a terminal.
//!
//! The encoder follows the terminal's state, as [`Modes`] reads it from the emulator:
//! xterm's forms where the program asked for nothing, the application cursor keys where
//! it asked for them, and the kitty keyboard protocol where it pushed its flags. Of those
//! flags it implements 1, every key with Ctrl or Option and Escape in the protocol's own
//! form, and 2, the repeat and the release of a key; see [`IMPLEMENTED_FLAGS`].
//!
//! A key that types text goes to the program through the view's input handler, which
//! also composes dead keys and the text of an input method: [`encode_key`] gives none for
//! it. A Cmd key never reaches a program: it gives none either.

use gpui_kit::Keystroke;

use crate::mouse::MouseMode;

/// Flags of the kitty keyboard protocol, as the bits a program pushes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyboardFlags(pub u8);

impl KeyboardFlags {
    /// None: xterm's forms.
    pub const NONE: KeyboardFlags = KeyboardFlags(0);
    /// 1: escape codes that can be told apart from typed text.
    pub const DISAMBIGUATE: KeyboardFlags = KeyboardFlags(1);
    /// 2: the repeat and the release of a key are sent too.
    pub const REPORT_EVENT_TYPES: KeyboardFlags = KeyboardFlags(2);
    /// 4: the key without Shift is sent beside the key with it.
    pub const REPORT_ALTERNATE_KEYS: KeyboardFlags = KeyboardFlags(4);
    /// 8: every key, text too, as an escape code.
    pub const REPORT_ALL_KEYS: KeyboardFlags = KeyboardFlags(8);
    /// 16: the text of a key beside its code.
    pub const REPORT_TEXT: KeyboardFlags = KeyboardFlags(16);

    /// Whether every flag of `other` is among these.
    pub fn contains(self, other: KeyboardFlags) -> bool {
        self.0 & other.0 == other.0
    }

    /// The flags both have.
    pub fn intersection(self, other: KeyboardFlags) -> KeyboardFlags {
        KeyboardFlags(self.0 & other.0)
    }
}

/// The flags the encoder implements, and the only ones the terminal grants a program
/// whatever it pushes: 1 and 2. Flag 4 needs the key a combination gives without Shift,
/// which the toolkit's keystroke does not carry.
pub const IMPLEMENTED_FLAGS: KeyboardFlags = KeyboardFlags(1 | 2);

/// What the program asked of the terminal that decides what input sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modes {
    /// The application cursor keys: arrows, Home and End as `ESC O` forms.
    pub app_cursor: bool,
    /// The kitty keyboard flags in force: never more than [`IMPLEMENTED_FLAGS`].
    pub keyboard: KeyboardFlags,
    /// A paste is sent between the two marks of bracketed paste.
    pub bracketed_paste: bool,
    /// Which mouse actions are reported.
    pub mouse: MouseMode,
    /// Mouse reports in the SGR form, not the old one.
    pub mouse_sgr: bool,
    /// Focus in and out are reported.
    pub focus_events: bool,
    /// The alternate screen is in use.
    pub alt_screen: bool,
    /// With the alternate screen, the wheel sends the arrow keys.
    pub alternate_scroll: bool,
}

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
    /// What Option does until the user's setting says otherwise: Meta on the U.S. layouts
    /// of macOS, characters on every other layout; Meta on every system but macOS, where
    /// Alt is always Meta.
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

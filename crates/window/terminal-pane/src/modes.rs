//! What a program asked of its terminal that decides what input sends: the kitty keyboard
//! flags, the cursor keys' form, bracketed paste, the mouse reports and the focus events.
//!
//! The emulator reads these from the program's output, and the encoders of keys and of the
//! mouse take them. This module uses no interface code, so that the emulator, which only
//! reads bytes and holds a screen, could serve a reader of terminals outside the window.

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

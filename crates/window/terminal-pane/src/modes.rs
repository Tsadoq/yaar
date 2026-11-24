//! Terminal modes a program set that decide what input sends.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyboardFlags(pub u8);

impl KeyboardFlags {
    pub const NONE: KeyboardFlags = KeyboardFlags(0);
    pub const DISAMBIGUATE: KeyboardFlags = KeyboardFlags(1);
    pub const REPORT_EVENT_TYPES: KeyboardFlags = KeyboardFlags(2);

    pub fn contains(self, other: KeyboardFlags) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn intersection(self, other: KeyboardFlags) -> KeyboardFlags {
        KeyboardFlags(self.0 & other.0)
    }
}

/// The flags the encoder implements, the only ones granted whatever a program pushes: flag 4 needs
/// the key without Shift, which the toolkit's keystroke does not carry.
pub const IMPLEMENTED_FLAGS: KeyboardFlags = KeyboardFlags(1 | 2);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modes {
    pub app_cursor: bool,
    pub keyboard: KeyboardFlags,
    pub bracketed_paste: bool,
    pub mouse: MouseMode,
    pub mouse_sgr: bool,
    pub focus_events: bool,
    pub alt_screen: bool,
    /// With the alternate screen, the wheel sends the arrow keys.
    pub alternate_scroll: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseMode {
    #[default]
    Off,
    Click,
    Drag,
    Motion,
}

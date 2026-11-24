//! What a key, a paste or a change of focus sends to the program, following [`crate::Modes`]. A key
//! that types text goes through the view's input handler: [`encode_key`] gives none for it.

use gpui_kit::Keystroke;

use crate::modes::{IMPLEMENTED_FLAGS, KeyboardFlags, Modes};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionKey {
    Meta,
    Characters,
}

impl OptionKey {
    /// Meta on macOS only when `layout_id` is exactly `com.apple.keylayout.US` (U.S. Extended and
    /// International type characters); Alt is always Meta elsewhere.
    pub fn for_layout(layout_id: &str, mac: bool) -> OptionKey {
        if !mac || layout_id == "com.apple.keylayout.US" {
            OptionKey::Meta
        } else {
            OptionKey::Characters
        }
    }
}

impl OptionKey {
    pub fn of_setting(setting: yaar_link::OptionKey, layout_id: &str, mac: bool) -> OptionKey {
        match setting {
            _ if !mac => OptionKey::Meta,
            yaar_link::OptionKey::Meta => OptionKey::Meta,
            yaar_link::OptionKey::Characters => OptionKey::Characters,
            yaar_link::OptionKey::ByLayout => OptionKey::for_layout(layout_id, mac),
        }
    }
}

const ESC: u8 = 0x1b;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Event {
    Press,
    Repeat,
    Release,
}

#[derive(Debug, PartialEq, Eq)]
enum Key {
    /// Bytes as they are: the key has no escape code that flag 2 could report on.
    Bytes(Vec<u8>),
    Cursor {
        letter: u8,
        modifiers: u32,
    },
    Function {
        number: u8,
        modifiers: u32,
    },
    Tilde {
        number: u32,
        modifiers: u32,
    },
    Code {
        code: u32,
        modifiers: u32,
        events: bool,
    },
}

fn modifier_parameter(keystroke: &Keystroke) -> u32 {
    let m = &keystroke.modifiers;
    1 + u32::from(m.shift) + 2 * u32::from(m.alt) + 4 * u32::from(m.control)
}

fn control_bytes(character: char) -> Vec<u8> {
    let byte = match character {
        'a'..='z' => character as u8 - b'a' + 1,
        'A'..='Z' => character as u8 - b'A' + 1,
        ' ' | '@' => 0,
        '[' => 27,
        '\\' => 28,
        ']' => 29,
        '^' => 30,
        '_' => 31,
        other => return other.to_string().into_bytes(),
    };
    vec![byte]
}

/// The key that types `symbol` without Shift on a U.S. keyboard; another layout's symbol has no
/// entry.
fn unshifted(symbol: char) -> Option<char> {
    const US_SHIFTED: [(char, char); 21] = [
        (')', '0'),
        ('!', '1'),
        ('@', '2'),
        ('#', '3'),
        ('$', '4'),
        ('%', '5'),
        ('^', '6'),
        ('&', '7'),
        ('*', '8'),
        ('(', '9'),
        ('~', '`'),
        ('_', '-'),
        ('+', '='),
        ('{', '['),
        ('}', ']'),
        ('|', '\\'),
        (':', ';'),
        ('"', '\''),
        ('<', ','),
        ('>', '.'),
        ('?', '/'),
    ];
    US_SHIFTED
        .iter()
        .find(|&&(shifted, _)| shifted == symbol)
        .map(|&(_, base)| base)
}

fn classify(keystroke: &Keystroke, modes: &Modes, option: OptionKey) -> Option<Key> {
    let m = &keystroke.modifiers;
    let flags = modes.keyboard.intersection(IMPLEMENTED_FLAGS);
    let kitty = flags.contains(KeyboardFlags::DISAMBIGUATE);
    let modifiers = modifier_parameter(keystroke);
    let key = keystroke.key.as_str();

    if m.platform {
        // Every Cmd key is the app's, except three that mean a line edit to a shell on macOS.
        let alone = !m.shift && !m.alt && !m.control;
        return match key {
            "left" if cfg!(target_os = "macos") && alone => Some(Key::Bytes(vec![0x01])),
            "right" if cfg!(target_os = "macos") && alone => Some(Key::Bytes(vec![0x05])),
            "backspace" if cfg!(target_os = "macos") && alone => Some(Key::Bytes(vec![0x15])),
            _ => None,
        };
    }

    let meta = |bytes: Vec<u8>| {
        if m.alt {
            let mut prefixed = vec![ESC];
            prefixed.extend(bytes);
            prefixed
        } else {
            bytes
        }
    };
    let modified = modifiers > 1;

    match key {
        "enter" => Some(if kitty && modified {
            Key::Code {
                code: 13,
                modifiers,
                events: false,
            }
        } else {
            Key::Bytes(meta(vec![b'\r']))
        }),
        "tab" => Some(if kitty && modified {
            Key::Code {
                code: 9,
                modifiers,
                events: false,
            }
        } else if m.shift {
            Key::Bytes(meta(b"\x1b[Z".to_vec()))
        } else {
            Key::Bytes(meta(vec![b'\t']))
        }),
        "backspace" => Some(if kitty && modified {
            Key::Code {
                code: 127,
                modifiers,
                events: false,
            }
        } else {
            Key::Bytes(meta(vec![if m.control { 0x08 } else { 0x7f }]))
        }),
        "escape" => Some(if kitty {
            Key::Code {
                code: 27,
                modifiers,
                events: true,
            }
        } else {
            Key::Bytes(meta(vec![ESC]))
        }),
        // Word-wise moves, which a shell reads as such in every state.
        "left" if m.alt && !m.shift && !m.control => Some(Key::Bytes(vec![ESC, b'b'])),
        "right" if m.alt && !m.shift && !m.control => Some(Key::Bytes(vec![ESC, b'f'])),
        "up" | "down" | "right" | "left" | "home" | "end" => Some(Key::Cursor {
            letter: match key {
                "up" => b'A',
                "down" => b'B',
                "right" => b'C',
                "left" => b'D',
                "home" => b'H',
                _ => b'F',
            },
            modifiers,
        }),
        "pageup" => Some(tilde(5, modifiers)),
        "pagedown" => Some(tilde(6, modifiers)),
        "delete" => Some(tilde(3, modifiers)),
        "insert" => Some(tilde(2, modifiers)),
        "f1" | "f2" | "f3" | "f4" => Some(Key::Function {
            number: key.as_bytes()[1] - b'0',
            modifiers,
        }),
        "f5" => Some(tilde(15, modifiers)),
        "f6" => Some(tilde(17, modifiers)),
        "f7" => Some(tilde(18, modifiers)),
        "f8" => Some(tilde(19, modifiers)),
        "f9" => Some(tilde(20, modifiers)),
        "f10" => Some(tilde(21, modifiers)),
        "f11" => Some(tilde(23, modifiers)),
        "f12" => Some(tilde(24, modifiers)),
        _ => {
            let character = match key {
                "space" => ' ',
                _ => {
                    let mut chars = key.chars();
                    match (chars.next(), chars.next()) {
                        (Some(character), None) => character,
                        _ => return None,
                    }
                }
            };
            let option_is_meta = m.alt && (option == OptionKey::Meta || m.control);
            if !m.control && !option_is_meta {
                return None;
            }
            if kitty {
                // The protocol names the key without Shift, and Shift a modifier. GPUI gives the
                // shifted symbol, so it is taken back to its key.
                let (code, modifiers) = match unshifted(character) {
                    Some(base) if !m.shift => (base, modifiers + 1),
                    Some(base) => (base, modifiers),
                    None => (
                        character.to_lowercase().next().unwrap_or(character),
                        modifiers,
                    ),
                };
                return Some(Key::Code {
                    code: code as u32,
                    modifiers,
                    events: true,
                });
            }
            let sent = if m.control {
                control_bytes(character)
            } else if m.shift && character.is_alphabetic() {
                let upper = character.to_uppercase().collect::<String>();
                upper.into_bytes()
            } else {
                character.to_string().into_bytes()
            };
            Some(Key::Bytes(meta(sent)))
        }
    }
}

fn tilde(number: u32, modifiers: u32) -> Key {
    Key::Tilde { number, modifiers }
}

fn csi(number: Option<u32>, modifiers: u32, event: Option<u8>, last: u8) -> Vec<u8> {
    let mut text = String::from("\x1b[");
    if let Some(number) = number {
        text.push_str(&number.to_string());
    }
    if modifiers > 1 || event.is_some() {
        text.push(';');
        text.push_str(&modifiers.to_string());
        if let Some(event) = event {
            text.push(':');
            text.push_str(&event.to_string());
        }
    }
    text.push(last as char);
    text.into_bytes()
}

impl Key {
    fn render(&self, event: Event, modes: &Modes) -> Vec<u8> {
        let flags = modes.keyboard.intersection(IMPLEMENTED_FLAGS);
        let kitty = flags.contains(KeyboardFlags::DISAMBIGUATE);
        let event = if flags.contains(KeyboardFlags::REPORT_EVENT_TYPES) {
            match event {
                Event::Press => None,
                Event::Repeat => Some(2),
                Event::Release => Some(3),
            }
        } else {
            None
        };
        match *self {
            Key::Bytes(ref bytes) => bytes.clone(),
            Key::Cursor { letter, modifiers } => {
                if modifiers == 1 && event.is_none() {
                    if modes.app_cursor {
                        vec![ESC, b'O', letter]
                    } else {
                        vec![ESC, b'[', letter]
                    }
                } else {
                    // The first parameter of a modified key is 1, so that the modifiers are the
                    // second.
                    csi(Some(1), modifiers, event, letter)
                }
            }
            Key::Function { number, modifiers } => {
                let letter = b'O' + number;
                if modifiers == 1 && event.is_none() && !kitty {
                    vec![ESC, b'O', letter]
                } else if number == 3 && (kitty || event.is_some()) {
                    // `CSI R` is the cursor report: the protocol writes F3 as 13 ~.
                    csi(Some(13), modifiers, event, b'~')
                } else if modifiers == 1 && event.is_none() {
                    csi(None, 1, None, letter)
                } else {
                    csi(Some(1), modifiers, event, letter)
                }
            }
            Key::Tilde { number, modifiers } => csi(Some(number), modifiers, event, b'~'),
            Key::Code {
                code,
                modifiers,
                events,
            } => csi(Some(code), modifiers, event.filter(|_| events), b'u'),
        }
    }
}

/// None for text (the input handler sends it), a Cmd key, or a key that sends nothing.
pub fn encode_key(
    keystroke: &Keystroke,
    modes: &Modes,
    option: OptionKey,
    repeat: bool,
) -> Option<Vec<u8>> {
    let event = if repeat { Event::Repeat } else { Event::Press };
    classify(keystroke, modes, option).map(|key| key.render(event, modes))
}

pub fn encode_release(keystroke: &Keystroke, modes: &Modes, option: OptionKey) -> Option<Vec<u8>> {
    let flags = modes.keyboard.intersection(IMPLEMENTED_FLAGS);
    if !flags.contains(KeyboardFlags::REPORT_EVENT_TYPES) {
        return None;
    }
    let key = classify(keystroke, modes, option)?;
    // A key sent as plain bytes has no escape code to carry the event, and the protocol
    // reports no release for Enter, Tab and Backspace.
    match key {
        Key::Bytes(_) | Key::Code { events: false, .. } => None,
        _ => Some(key.render(Event::Release, modes)),
    }
}

/// Text that holds the closing mark loses it, so that a paste cannot end itself early.
pub fn encode_paste(text: &str, modes: &Modes) -> Vec<u8> {
    const OPEN: &[u8] = b"\x1b[200~";
    const CLOSE: &[u8] = b"\x1b[201~";
    let mut bytes = text.replace("\r\n", "\r").replace('\n', "\r").into_bytes();
    if !modes.bracketed_paste {
        return bytes;
    }
    // Taking one mark out can join the pieces on either side into another.
    while let Some(at) = bytes
        .windows(CLOSE.len())
        .position(|window| window == CLOSE)
    {
        bytes.drain(at..at + CLOSE.len());
    }
    let mut marked = OPEN.to_vec();
    marked.extend(bytes);
    marked.extend_from_slice(CLOSE);
    marked
}

pub fn encode_focus(focused: bool, modes: &Modes) -> Option<Vec<u8>> {
    modes
        .focus_events
        .then(|| vec![ESC, b'[', if focused { b'I' } else { b'O' }])
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEGACY: Modes = Modes {
        app_cursor: false,
        keyboard: KeyboardFlags::NONE,
        bracketed_paste: false,
        mouse: crate::modes::MouseMode::Off,
        mouse_sgr: false,
        focus_events: false,
        alt_screen: false,
        alternate_scroll: false,
    };
    const APP_CURSOR: Modes = Modes {
        app_cursor: true,
        ..LEGACY
    };
    const KITTY: Modes = Modes {
        keyboard: KeyboardFlags(1),
        ..LEGACY
    };
    const KITTY_EVENTS: Modes = Modes {
        keyboard: KeyboardFlags(3),
        ..LEGACY
    };
    const EVENTS_ONLY: Modes = Modes {
        keyboard: KeyboardFlags(2),
        ..LEGACY
    };

    fn key(source: &'static str) -> Keystroke {
        Keystroke::parse(source).expect("a keystroke")
    }

    fn shifted(source: &'static str) -> Keystroke {
        let mut keystroke = key(source);
        keystroke.modifiers.shift = true;
        keystroke
    }

    fn sent(keystroke: &Keystroke, modes: &Modes) -> Option<Vec<u8>> {
        encode_key(keystroke, modes, OptionKey::Meta, false)
    }

    #[track_caller]
    fn assert_sends(modes: &Modes, rows: &[(&'static str, Option<&str>)]) {
        for (source, expected) in rows {
            let got = sent(&key(source), modes).map(|bytes| show(&bytes));
            assert_eq!(got.as_deref(), *expected, "{source} under {modes:?}");
        }
    }

    fn show(bytes: &[u8]) -> String {
        bytes
            .iter()
            .map(|&byte| match byte {
                0x1b => "E".to_string(),
                0x20..=0x7e => (byte as char).to_string(),
                _ => format!("<{byte:02x}>"),
            })
            .collect()
    }

    #[test]
    fn option_is_meta_on_the_us_layout_and_characters_on_the_others() {
        let us = "com.apple.keylayout.US";
        assert_eq!(OptionKey::for_layout(us, true), OptionKey::Meta);
        for other in [
            "com.apple.keylayout.ABC",
            "com.apple.keylayout.USExtended",
            "com.apple.keylayout.USInternational-PC",
            "com.apple.keylayout.German",
            "zed.keyboard.example",
            "",
        ] {
            assert_eq!(
                OptionKey::for_layout(other, true),
                OptionKey::Characters,
                "{other}"
            );
        }
        for layout in [us, "com.apple.keylayout.ABC", "", "xkb:de"] {
            assert_eq!(
                OptionKey::for_layout(layout, false),
                OptionKey::Meta,
                "{layout}"
            );
        }
    }

    #[test]
    fn the_setting_decides_option_on_macos_and_with_none_made_the_layout_does() {
        use yaar_link::OptionKey as Setting;
        let (us, other) = ("com.apple.keylayout.US", "com.apple.keylayout.German");
        for layout in [us, other] {
            assert_eq!(
                OptionKey::of_setting(Setting::Meta, layout, true),
                OptionKey::Meta,
                "{layout}"
            );
            assert_eq!(
                OptionKey::of_setting(Setting::Characters, layout, true),
                OptionKey::Characters,
                "{layout}"
            );
        }
        assert_eq!(
            OptionKey::of_setting(Setting::ByLayout, us, true),
            OptionKey::Meta
        );
        assert_eq!(
            OptionKey::of_setting(Setting::ByLayout, other, true),
            OptionKey::Characters
        );
    }

    #[test]
    fn option_is_meta_off_macos_whatever_the_setting_and_the_layout() {
        use yaar_link::OptionKey as Setting;
        for setting in [Setting::ByLayout, Setting::Meta, Setting::Characters] {
            for layout in ["com.apple.keylayout.US", "com.apple.keylayout.German", ""] {
                assert_eq!(
                    OptionKey::of_setting(setting, layout, false),
                    OptionKey::Meta,
                    "{setting:?} {layout}"
                );
            }
        }
    }

    #[test]
    fn a_key_that_types_text_is_left_to_the_input_handler() {
        assert_sends(
            &LEGACY,
            &[
                ("a->a", None),
                ("shift-A->A", None),
                ("1->1", None),
                ("shift-!->!", None),
                ("space-> ", None),
                ("shift-space-> ", None),
                ("a", None),
                ("alt-e->é", Some("Ee")),
            ],
        );
        assert_sends(
            &KITTY,
            &[("a->a", None), ("shift-A->A", None), ("space-> ", None)],
        );
        assert_sends(&KITTY_EVENTS, &[("a->a", None), ("1->1", None)]);
    }

    #[test]
    fn keys_that_are_no_keys_send_nothing() {
        assert_sends(
            &KITTY_EVENTS,
            &[
                ("shift", None),
                ("ctrl-shift", None),
                ("capslock", None),
                ("alt", None),
            ],
        );
    }

    #[test]
    fn ctrl_with_a_letter_sends_its_control_byte() {
        assert_sends(
            &LEGACY,
            &[
                ("ctrl-a", Some("<01>")),
                ("ctrl-c", Some("<03>")),
                ("ctrl-z", Some("<1a>")),
                ("ctrl-shift-a", Some("<01>")),
                ("ctrl-alt-a", Some("E<01>")),
                ("ctrl-space", Some("<00>")),
                ("ctrl-[", Some("E")),
                ("ctrl-\\", Some("<1c>")),
                ("ctrl-]", Some("<1d>")),
            ],
        );
        // Shift folds into the symbol: Ctrl-Shift-2 is Ctrl with @, Ctrl-Shift-6 with ^.
        for (symbol, byte) in [("ctrl-@", "<00>"), ("ctrl-^", "<1e>"), ("ctrl-_", "<1f>")] {
            let got = sent(&shifted(symbol), &LEGACY).map(|bytes| show(&bytes));
            assert_eq!(got.as_deref(), Some(byte), "{symbol}");
        }
    }

    #[test]
    fn ctrl_with_a_digit_or_other_sign_sends_the_character() {
        assert_sends(
            &LEGACY,
            &[
                ("ctrl-1", Some("1")),
                ("ctrl-0", Some("0")),
                ("ctrl-;", Some(";")),
                ("ctrl-,", Some(",")),
                ("ctrl-/", Some("/")),
                ("ctrl-=", Some("=")),
                ("ctrl-`", Some("`")),
                ("ctrl-alt-1", Some("E1")),
            ],
        );
        let got = sent(&shifted("ctrl-!"), &LEGACY).map(|bytes| show(&bytes));
        assert_eq!(got.as_deref(), Some("!"));
    }

    #[test]
    fn alt_as_meta_puts_an_escape_before_the_key() {
        assert_sends(
            &LEGACY,
            &[
                ("alt-b", Some("Eb")),
                ("alt-shift-b", Some("EB")),
                ("alt-1", Some("E1")),
                ("alt-space", Some("E ")),
                ("alt-shift-space", Some("E ")),
                ("alt-.", Some("E.")),
            ],
        );
        let got = sent(&shifted("alt-!"), &LEGACY).map(|bytes| show(&bytes));
        assert_eq!(got.as_deref(), Some("E!"));
    }

    #[test]
    fn alt_that_types_characters_is_text_and_not_the_encoders() {
        for modes in [LEGACY, KITTY, KITTY_EVENTS] {
            for source in ["alt-b", "alt-e->é", "alt-shift-b->B", "alt-1", "alt-space"] {
                assert_eq!(
                    encode_key(&key(source), &modes, OptionKey::Characters, false),
                    None,
                    "{source} under {modes:?}"
                );
            }
        }
        let characters = |source: &'static str, modes: &Modes| {
            encode_key(&key(source), modes, OptionKey::Characters, false).map(|b| show(&b))
        };
        assert_eq!(characters("alt-enter", &LEGACY).as_deref(), Some("E<0d>"));
        assert_eq!(characters("alt-up", &LEGACY).as_deref(), Some("E[1;3A"));
        assert_eq!(characters("alt-left", &LEGACY).as_deref(), Some("Eb"));
        assert_eq!(characters("ctrl-alt-a", &LEGACY).as_deref(), Some("E<01>"));
        assert_eq!(characters("ctrl-alt-a", &KITTY).as_deref(), Some("E[97;7u"));
    }

    #[test]
    fn return_tab_backspace_and_escape_in_the_legacy_state() {
        assert_sends(
            &LEGACY,
            &[
                ("enter", Some("<0d>")),
                ("shift-enter", Some("<0d>")),
                ("ctrl-enter", Some("<0d>")),
                ("alt-enter", Some("E<0d>")),
                ("alt-shift-enter", Some("E<0d>")),
                ("tab", Some("<09>")),
                ("shift-tab", Some("E[Z")),
                ("ctrl-tab", Some("<09>")),
                ("alt-tab", Some("E<09>")),
                ("backspace", Some("<7f>")),
                ("shift-backspace", Some("<7f>")),
                ("ctrl-backspace", Some("<08>")),
                ("alt-backspace", Some("E<7f>")),
                ("escape", Some("E")),
                ("shift-escape", Some("E")),
                ("ctrl-escape", Some("E")),
                ("alt-escape", Some("EE")),
            ],
        );
    }

    #[test]
    fn named_keys_are_the_encoders_even_when_they_carry_text() {
        for (name, text, expected) in [
            ("enter", "\n", "<0d>"),
            ("tab", "\t", "<09>"),
            ("backspace", "\u{8}", "<7f>"),
            ("escape", "\u{1b}", "E"),
        ] {
            let keystroke = Keystroke {
                key: name.to_string(),
                key_char: Some(text.to_string()),
                ..Keystroke::default()
            };
            assert_eq!(
                sent(&keystroke, &LEGACY).map(|b| show(&b)).as_deref(),
                Some(expected),
                "{name}"
            );
        }
    }

    #[test]
    fn the_cursor_keys_follow_the_cursor_mode_and_carry_their_modifiers() {
        assert_sends(
            &LEGACY,
            &[
                ("up", Some("E[A")),
                ("down", Some("E[B")),
                ("right", Some("E[C")),
                ("left", Some("E[D")),
                ("home", Some("E[H")),
                ("end", Some("E[F")),
                ("shift-up", Some("E[1;2A")),
                ("alt-up", Some("E[1;3A")),
                ("ctrl-up", Some("E[1;5A")),
                ("ctrl-shift-up", Some("E[1;6A")),
                ("ctrl-alt-shift-up", Some("E[1;8A")),
                ("ctrl-home", Some("E[1;5H")),
                ("shift-end", Some("E[1;2F")),
            ],
        );
        assert_sends(
            &APP_CURSOR,
            &[
                ("up", Some("EOA")),
                ("down", Some("EOB")),
                ("right", Some("EOC")),
                ("left", Some("EOD")),
                ("home", Some("EOH")),
                ("end", Some("EOF")),
                ("shift-up", Some("E[1;2A")),
                ("ctrl-end", Some("E[1;5F")),
            ],
        );
    }

    #[test]
    fn option_left_and_right_move_by_a_word_in_every_state() {
        for modes in [LEGACY, APP_CURSOR, KITTY, KITTY_EVENTS] {
            assert_sends(
                &modes,
                &[("alt-left", Some("Eb")), ("alt-right", Some("Ef"))],
            );
            assert_sends(
                &modes,
                &[
                    ("alt-shift-left", Some("E[1;4D")),
                    ("ctrl-alt-right", Some("E[1;7C")),
                ],
            );
        }
    }

    #[test]
    fn page_keys_delete_and_insert_are_tilde_forms() {
        assert_sends(
            &LEGACY,
            &[
                ("pageup", Some("E[5~")),
                ("pagedown", Some("E[6~")),
                ("delete", Some("E[3~")),
                ("insert", Some("E[2~")),
                ("shift-pageup", Some("E[5;2~")),
                ("alt-pagedown", Some("E[6;3~")),
                ("ctrl-delete", Some("E[3;5~")),
                ("ctrl-shift-delete", Some("E[3;6~")),
            ],
        );
    }

    #[test]
    fn function_keys_in_the_legacy_state() {
        assert_sends(
            &LEGACY,
            &[
                ("f1", Some("EOP")),
                ("f2", Some("EOQ")),
                ("f3", Some("EOR")),
                ("f4", Some("EOS")),
                ("shift-f1", Some("E[1;2P")),
                ("ctrl-f3", Some("E[1;5R")),
                ("alt-f4", Some("E[1;3S")),
                ("f5", Some("E[15~")),
                ("f6", Some("E[17~")),
                ("f7", Some("E[18~")),
                ("f8", Some("E[19~")),
                ("f9", Some("E[20~")),
                ("f10", Some("E[21~")),
                ("f11", Some("E[23~")),
                ("f12", Some("E[24~")),
                ("ctrl-f5", Some("E[15;5~")),
                ("shift-f12", Some("E[24;2~")),
            ],
        );
        assert_sends(&APP_CURSOR, &[("f1", Some("EOP")), ("f5", Some("E[15~"))]);
    }

    #[test]
    fn under_flag_1_keys_with_ctrl_or_alt_are_escape_codes() {
        assert_sends(
            &KITTY,
            &[
                ("ctrl-a", Some("E[97;5u")),
                ("ctrl-shift-a", Some("E[97;6u")),
                ("alt-a", Some("E[97;3u")),
                ("alt-shift-a", Some("E[97;4u")),
                ("ctrl-alt-a", Some("E[97;7u")),
                ("ctrl-space", Some("E[32;5u")),
                ("alt-space", Some("E[32;3u")),
                ("ctrl-1", Some("E[49;5u")),
                ("ctrl-;", Some("E[59;5u")),
                ("alt-.", Some("E[46;3u")),
            ],
        );
    }

    const US_SHIFTED: [(char, char); 21] = [
        (')', '0'),
        ('!', '1'),
        ('@', '2'),
        ('#', '3'),
        ('$', '4'),
        ('%', '5'),
        ('^', '6'),
        ('&', '7'),
        ('*', '8'),
        ('(', '9'),
        ('~', '`'),
        ('_', '-'),
        ('+', '='),
        ('{', '['),
        ('}', ']'),
        ('|', '\\'),
        (':', ';'),
        ('"', '\''),
        ('<', ','),
        ('>', '.'),
        ('?', '/'),
    ];

    fn symbol_keys(modifiers: &str, symbol: char) -> [Keystroke; 2] {
        let mut folded = key("a");
        folded.key = symbol.to_string();
        folded.modifiers.control = modifiers.contains("ctrl");
        folded.modifiers.alt = modifiers.contains("alt");
        let mut set = folded.clone();
        set.modifiers.shift = true;
        [folded, set]
    }

    #[test]
    fn under_flag_1_a_symbol_typed_with_shift_is_its_key_and_shift_among_the_modifiers() {
        for (symbol, base) in US_SHIFTED {
            for (modifiers, parameter) in [("ctrl", 6), ("alt", 4), ("ctrl-alt", 8)] {
                for keystroke in symbol_keys(modifiers, symbol) {
                    assert_eq!(
                        sent(&keystroke, &KITTY).map(|bytes| show(&bytes)),
                        Some(format!("E[{};{parameter}u", base as u32)),
                        "{keystroke:?} under flag 1"
                    );
                }
            }
        }
        for keystroke in symbol_keys("ctrl", ')') {
            assert_eq!(
                encode_release(&keystroke, &KITTY_EVENTS, OptionKey::Meta).map(|b| show(&b)),
                Some("E[48;6:3u".to_string())
            );
            assert_eq!(
                encode_key(&keystroke, &KITTY_EVENTS, OptionKey::Meta, true).map(|b| show(&b)),
                Some("E[48;6:2u".to_string())
            );
        }
    }

    #[test]
    fn under_flag_1_a_symbol_with_no_shift_in_it_stays_as_it_is() {
        assert_sends(
            &KITTY,
            &[
                ("ctrl-0", Some("E[48;5u")),
                ("ctrl--", Some("E[45;5u")),
                ("ctrl-=", Some("E[61;5u")),
                ("alt-/", Some("E[47;3u")),
                ("ctrl-shift-0", Some("E[48;6u")),
                ("ctrl-shift-a", Some("E[97;6u")),
                ("ctrl-£", Some("E[163;5u")),
                ("ctrl-§", Some("E[167;5u")),
            ],
        );
    }

    #[test]
    fn a_symbol_typed_with_shift_is_unchanged_where_the_protocol_is_not_in_force() {
        for (symbol, _) in US_SHIFTED {
            for keystroke in symbol_keys("ctrl", symbol) {
                assert_eq!(
                    sent(&keystroke, &LEGACY).map(|bytes| show(&bytes)),
                    Some(show(&control_bytes(symbol))),
                    "{keystroke:?} in the legacy state"
                );
            }
        }
    }

    #[test]
    fn under_flag_1_escape_enter_tab_and_backspace_take_their_codes() {
        assert_sends(
            &KITTY,
            &[
                ("escape", Some("E[27u")),
                ("shift-escape", Some("E[27;2u")),
                ("alt-escape", Some("E[27;3u")),
                ("ctrl-escape", Some("E[27;5u")),
                ("enter", Some("<0d>")),
                ("shift-enter", Some("E[13;2u")),
                ("ctrl-enter", Some("E[13;5u")),
                ("alt-enter", Some("E[13;3u")),
                ("tab", Some("<09>")),
                ("shift-tab", Some("E[9;2u")),
                ("ctrl-tab", Some("E[9;5u")),
                ("alt-tab", Some("E[9;3u")),
                ("backspace", Some("<7f>")),
                ("shift-backspace", Some("E[127;2u")),
                ("ctrl-backspace", Some("E[127;5u")),
                ("alt-backspace", Some("E[127;3u")),
            ],
        );
    }

    #[test]
    fn under_flag_1_function_keys_one_to_four_take_the_csi_forms() {
        assert_sends(
            &KITTY,
            &[
                ("f1", Some("E[P")),
                ("f2", Some("E[Q")),
                ("f3", Some("E[13~")),
                ("f4", Some("E[S")),
                ("shift-f1", Some("E[1;2P")),
                ("shift-f3", Some("E[13;2~")),
                ("ctrl-f4", Some("E[1;5S")),
                ("f5", Some("E[15~")),
                ("up", Some("E[A")),
                ("ctrl-up", Some("E[1;5A")),
                ("pageup", Some("E[5~")),
                ("delete", Some("E[3~")),
            ],
        );
        let modes = Modes {
            app_cursor: true,
            ..KITTY
        };
        assert_sends(&modes, &[("up", Some("EOA")), ("f1", Some("E[P"))]);
    }

    #[test]
    fn under_flag_2_a_repeat_is_marked_with_event_type_2() {
        let repeat = |source: &'static str, modes: &Modes| {
            encode_key(&key(source), modes, OptionKey::Meta, true).map(|b| show(&b))
        };
        for (source, expected) in [
            ("ctrl-a", "E[97;5:2u"),
            ("escape", "E[27;1:2u"),
            ("up", "E[1;1:2A"),
            ("shift-left", "E[1;2:2D"),
            ("f1", "E[1;1:2P"),
            ("f3", "E[13;1:2~"),
            ("f5", "E[15;1:2~"),
            ("delete", "E[3;1:2~"),
            ("enter", "<0d>"),
            ("shift-enter", "E[13;2u"),
            ("backspace", "<7f>"),
            ("shift-tab", "E[9;2u"),
            ("alt-left", "Eb"),
        ] {
            assert_eq!(
                repeat(source, &KITTY_EVENTS).as_deref(),
                Some(expected),
                "{source}"
            );
        }
        for source in ["ctrl-a", "escape", "up", "f3"] {
            assert_eq!(
                repeat(source, &KITTY),
                sent(&key(source), &KITTY).map(|b| show(&b)),
                "{source}"
            );
        }
        assert_sends(
            &KITTY_EVENTS,
            &[
                ("ctrl-a", Some("E[97;5u")),
                ("up", Some("E[A")),
                ("escape", Some("E[27u")),
            ],
        );
    }

    #[test]
    fn under_flag_2_the_release_of_a_key_the_encoder_sent_is_event_type_3() {
        let release = |source: &'static str, modes: &Modes| {
            encode_release(&key(source), modes, OptionKey::Meta).map(|b| show(&b))
        };
        for (source, expected) in [
            ("ctrl-a", "E[97;5:3u"),
            ("ctrl-shift-a", "E[97;6:3u"),
            ("alt-b", "E[98;3:3u"),
            ("escape", "E[27;1:3u"),
            ("alt-escape", "E[27;3:3u"),
            ("up", "E[1;1:3A"),
            ("shift-home", "E[1;2:3H"),
            ("f1", "E[1;1:3P"),
            ("f3", "E[13;1:3~"),
            ("f5", "E[15;1:3~"),
            ("pageup", "E[5;1:3~"),
            ("delete", "E[3;1:3~"),
            ("ctrl-delete", "E[3;5:3~"),
        ] {
            assert_eq!(
                release(source, &KITTY_EVENTS).as_deref(),
                Some(expected),
                "{source}"
            );
        }
    }

    #[test]
    fn a_key_the_encoder_did_not_send_has_no_release() {
        let release = |source: &'static str, modes: &Modes| {
            encode_release(&key(source), modes, OptionKey::Meta)
        };
        for source in [
            "a->a",
            "space-> ",
            "shift-A->A",
            "enter",
            "shift-enter",
            "tab",
            "shift-tab",
            "backspace",
            "ctrl-backspace",
            "alt-left",
            "alt-right",
            "cmd-a",
            "cmd-left",
            "shift",
        ] {
            assert_eq!(release(source, &KITTY_EVENTS), None, "{source}");
        }
        for modes in [LEGACY, KITTY, APP_CURSOR] {
            for source in ["ctrl-a", "escape", "up", "f1", "delete"] {
                assert_eq!(release(source, &modes), None, "{source} under {modes:?}");
            }
        }
        assert_eq!(
            encode_release(&key("alt-b"), &KITTY_EVENTS, OptionKey::Characters),
            None
        );
    }

    #[test]
    fn flag_2_alone_reports_the_keys_that_are_escape_codes_in_any_case() {
        let release = |source: &'static str| {
            encode_release(&key(source), &EVENTS_ONLY, OptionKey::Meta).map(|b| show(&b))
        };
        assert_eq!(release("up").as_deref(), Some("E[1;1:3A"));
        assert_eq!(release("f3").as_deref(), Some("E[13;1:3~"));
        assert_eq!(release("ctrl-a"), None);
        assert_eq!(release("escape"), None);
        assert_sends(
            &EVENTS_ONLY,
            &[
                ("ctrl-a", Some("<01>")),
                ("escape", Some("E")),
                ("up", Some("E[A")),
            ],
        );
    }

    #[test]
    fn a_cmd_key_sends_nothing_but_the_three_line_edits_on_macos() {
        assert_sends(
            &KITTY_EVENTS,
            &[
                ("cmd-a", None),
                ("cmd-c", None),
                ("cmd-1", None),
                ("cmd-up", None),
                ("cmd-down", None),
                ("cmd-home", None),
                ("cmd-end", None),
                ("cmd-pageup", None),
                ("cmd-shift-left", None),
                ("cmd-alt-left", None),
                ("cmd-enter", None),
                ("cmd-escape", None),
            ],
        );
        let (left, right, backspace) = if cfg!(target_os = "macos") {
            (Some("<01>"), Some("<05>"), Some("<15>"))
        } else {
            (None, None, None)
        };
        for modes in [LEGACY, APP_CURSOR, KITTY, KITTY_EVENTS] {
            assert_sends(
                &modes,
                &[
                    ("cmd-left", left),
                    ("cmd-right", right),
                    ("cmd-backspace", backspace),
                ],
            );
        }
    }

    #[test]
    fn a_paste_ends_its_lines_as_return_does() {
        assert_eq!(
            encode_paste("one\ntwo\r\nthree\rfour", &LEGACY),
            b"one\rtwo\rthree\rfour"
        );
        assert_eq!(encode_paste("", &LEGACY), b"");
        assert_eq!(encode_paste("é\u{4f60}", &LEGACY), "é\u{4f60}".as_bytes());
    }

    #[test]
    fn a_bracketed_paste_is_wrapped_in_its_marks() {
        let modes = Modes {
            bracketed_paste: true,
            ..LEGACY
        };
        assert_eq!(encode_paste("a\nb", &modes), b"\x1b[200~a\rb\x1b[201~");
        assert_eq!(encode_paste("", &modes), b"\x1b[200~\x1b[201~");
    }

    #[test]
    fn a_paste_cannot_end_itself_early() {
        let modes = Modes {
            bracketed_paste: true,
            ..LEGACY
        };
        assert_eq!(
            encode_paste("a\x1b[201~b\x1b[201~", &modes),
            b"\x1b[200~ab\x1b[201~"
        );
        assert_eq!(
            encode_paste("\x1b[2\x1b[201~01~x", &modes),
            b"\x1b[200~x\x1b[201~"
        );
        assert_eq!(
            encode_paste("\x1b[200~x", &modes),
            b"\x1b[200~\x1b[200~x\x1b[201~"
        );
    }

    #[test]
    fn focus_is_told_only_to_a_program_that_asked() {
        assert_eq!(encode_focus(true, &LEGACY), None);
        assert_eq!(encode_focus(false, &LEGACY), None);
        let modes = Modes {
            focus_events: true,
            ..LEGACY
        };
        assert_eq!(encode_focus(true, &modes), Some(b"\x1b[I".to_vec()));
        assert_eq!(encode_focus(false, &modes), Some(b"\x1b[O".to_vec()));
    }
}

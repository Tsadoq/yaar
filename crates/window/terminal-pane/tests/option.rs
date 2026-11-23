//! Component test (TERM-07): what the Option key does in a terminal, from the user's
//! setting, the keyboard layout and the system.
//!
//! The pane runs on GPUI's test platform with the app prepared as the Window program
//! prepares it. The test plays the Keeper: it tells the setting as the Keeper tells it
//! (`Fact::SettingChanged`) and reads the bytes the program of the terminal receives
//! (`TestKeeper::input_until`). Every key is followed by a sentinel, text whose bytes are
//! known, and what comes before the sentinel is what the key sent: a key that sends
//! nothing shows as the sentinel arriving first.
//!
//! The test platform has one keyboard layout, `zed.keyboard.example`, which is not
//! `com.apple.keylayout.US`: with no setting Option types characters on macOS. That the U.S.
//! layout makes it Meta is shown on the rule itself, by the unit tests of `keys.rs`, since
//! the platform cannot change its layout.
//!
//! The macOS tests and the test of the other systems differ only in the attributes on the
//! test functions: the bodies are shared and compile everywhere.

mod common;

use gpui_kit::{Keystroke, TestAppContext, VisualTestContext};
use yaar_link::{Fact, OptionKey, Setting, TerminalId};

use common::{Panes, open_panes, terminal, wait_drawn, wait_until};

/// A pane with one terminal that has the focus, and the sentinels typed so far.
struct Typing {
    panes: Panes,
    cx: VisualTestContext,
    id: TerminalId,
    sentinels: usize,
}

impl Typing {
    fn new(cx: &mut TestAppContext) -> Typing {
        let (mut panes, mut cx) = open_panes(cx);
        let id = terminal('1');
        panes.open_terminal(&id, &mut cx);
        wait_drawn(&mut cx, "the terminal to have the focus", |cx| {
            panes.focused_terminal(cx).as_ref() == Some(&id)
        });
        Typing {
            panes,
            cx,
            id,
            sentinels: 0,
        }
    }

    /// Tells the Window the Keeper's setting and waits until the model holds it.
    fn set(&mut self, key: OptionKey) {
        self.panes
            .keeper
            .tell(Fact::SettingChanged(Setting::OptionKey(key)));
        wait_until(&mut self.cx, "the model to hold the setting", |cx| {
            self.panes
                .models
                .settings
                .read_with(cx, |list, _| list.option_key() == key)
        });
    }

    /// Presses Option with F as the platform delivers it, with the character the layout
    /// gives that key under Option as its text, and gives what the program received.
    fn alt_f(&mut self) -> Vec<u8> {
        self.cx.update(|window, cx| {
            let keystroke = Keystroke::parse_with_os("alt-f->ƒ", cx.platform_info().os)
                .expect("a key written as the platform delivers it");
            window.dispatch_keystroke(keystroke, cx);
        });
        self.sentinel()
    }

    /// Types the next sentinel and reads what the program received before it.
    fn sentinel(&mut self) -> Vec<u8> {
        self.sentinels += 1;
        let sentinel = format!("<s{}>", self.sentinels);
        self.cx.simulate_input(&sentinel);
        self.panes
            .keeper
            .input_until(&self.id, sentinel.as_bytes(), &mut self.cx)
    }
}

/// What the key under Option types on the layout of the test platform.
const CHARACTER: &[u8] = "ƒ".as_bytes();

/// Escape, then the letter: Option as Meta.
const META: &[u8] = b"\x1bf";

#[cfg(target_os = "macos")]
#[gpui_kit::test]
fn with_no_setting_option_types_the_characters_the_layout_gives_on_macos(cx: &mut TestAppContext) {
    let mut typing = Typing::new(cx);
    // The layout of the test platform is not the U.S. one.
    assert_eq!(typing.alt_f(), CHARACTER);
}

#[cfg(target_os = "macos")]
#[gpui_kit::test]
fn option_set_to_meta_sends_escape_and_the_key_on_every_layout_on_macos(cx: &mut TestAppContext) {
    let mut typing = Typing::new(cx);
    typing.set(OptionKey::Meta);
    assert_eq!(typing.alt_f(), META);
}

#[cfg(target_os = "macos")]
#[gpui_kit::test]
fn option_set_to_characters_types_the_characters_of_the_layout_on_macos(cx: &mut TestAppContext) {
    let mut typing = Typing::new(cx);
    typing.set(OptionKey::Meta);
    assert_eq!(typing.alt_f(), META);
    typing.set(OptionKey::Characters);
    assert_eq!(typing.alt_f(), CHARACTER);
}

#[cfg(target_os = "macos")]
#[gpui_kit::test]
fn option_set_back_to_the_layout_follows_the_layout_again_on_macos(cx: &mut TestAppContext) {
    let mut typing = Typing::new(cx);
    typing.set(OptionKey::Meta);
    assert_eq!(typing.alt_f(), META);
    typing.set(OptionKey::ByLayout);
    assert_eq!(typing.alt_f(), CHARACTER);
}

#[cfg(not(target_os = "macos"))]
#[gpui_kit::test]
fn option_is_always_meta_off_macos_whatever_the_setting(cx: &mut TestAppContext) {
    let mut typing = Typing::new(cx);
    assert_eq!(typing.alt_f(), META);
    typing.set(OptionKey::Characters);
    assert_eq!(typing.alt_f(), META);
    typing.set(OptionKey::ByLayout);
    assert_eq!(typing.alt_f(), META);
}

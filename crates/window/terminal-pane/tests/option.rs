//! What Option sends in a terminal, by setting, layout and system. Each key is followed by a
//! sentinel: a key that sends nothing shows as the sentinel arriving first. The test platform's
//! layout is not U.S., so on macOS with no setting Option types characters.

mod common;

use gpui_kit::{Keystroke, TestAppContext, VisualTestContext};
use yaar_link::{Fact, OptionKey, Setting, TerminalId};

use common::{Panes, open_panes, terminal, wait_drawn, wait_until};

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

    fn alt_f(&mut self) -> Vec<u8> {
        self.cx.update(|window, cx| {
            let keystroke = Keystroke::parse_with_os("alt-f->ƒ", cx.platform_info().os)
                .expect("a key written as the platform delivers it");
            window.dispatch_keystroke(keystroke, cx);
        });
        self.sentinel()
    }

    fn sentinel(&mut self) -> Vec<u8> {
        self.sentinels += 1;
        let sentinel = format!("<s{}>", self.sentinels);
        self.cx.simulate_input(&sentinel);
        self.panes
            .keeper
            .input_until(&self.id, sentinel.as_bytes(), &mut self.cx)
    }
}

#[cfg(target_os = "macos")]
const CHARACTER: &[u8] = "ƒ".as_bytes();

const META: &[u8] = b"\x1bf";

#[cfg(target_os = "macos")]
#[gpui_kit::test]
fn with_no_setting_option_types_the_characters_the_layout_gives_on_macos(cx: &mut TestAppContext) {
    let mut typing = Typing::new(cx);
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

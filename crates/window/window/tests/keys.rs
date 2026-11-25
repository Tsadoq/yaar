//! The keys the app keeps for itself, over the built Keeper with a real shell. A sentinel typed
//! after each key shows what the key sent (nothing, if the sentinel arrives first).

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use gpui_kit::{Action, Keystroke, TestAppContext, VisualTestContext};
use yaar_frame::{Control, PROGRAM_CONTEXT};
use yaar_link::TerminalId;
use yaar_terminal_pane::{NewAgentTerminal, NewTerminal};

use common::{Composed, key_of, open_composed, wait_until};

const SYSTEM: &str = if cfg!(target_os = "macos") {
    "macos"
} else {
    "linux"
};

/// The bytes a terminal in the legacy state receives for each key, read from the terminal pane's
/// own test.
const LEGACY: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../terminal-pane/tests/matrix/legacy.tsv"
));

fn documented() -> BTreeSet<String> {
    let mut keys: Vec<&str> = vec![
        "ctrl-tab",
        "ctrl-shift-tab",
        "shift-pageup",
        "shift-pagedown",
    ];
    if cfg!(target_os = "macos") {
        keys.extend([
            "cmd-c",
            "cmd-v",
            "cmd-o",
            "cmd-shift-b",
            "cmd-b",
            "cmd-alt-b",
            "cmd-alt-down",
            "cmd-alt-up",
            "cmd-alt-p",
            "cmd-alt-n",
            "cmd-alt-q",
            "cmd-t",
            "cmd-shift-w",
            "cmd-]",
            "cmd-[",
            "cmd-d",
            "cmd-q",
            "cmd-alt-v",
            "cmd-alt-x",
            "cmd-alt-a",
            "cmd-shift-t",
            "cmd-alt-c",
            "cmd-shift-r",
            "cmd-alt-e",
            "cmd-alt-shift-e",
            "cmd-,",
            "cmd-shift-n",
            "cmd-k",
            "cmd-a",
            "cmd-home",
            "cmd-end",
            "cmd-up",
            "cmd-down",
            "cmd-w",
            "cmd-h",
            "cmd-alt-h",
            "cmd-m",
        ]);
    } else {
        keys.extend(["ctrl-shift-c", "ctrl-shift-v"]);
    }
    keys.into_iter().map(str::to_owned).collect()
}

#[test]
fn the_documented_list_of_kept_keys_is_what_the_window_keeps() {
    let kept: BTreeSet<String> = yaar_window::kept_keys().iter().map(Control::key).collect();
    assert_eq!(kept, documented());
    if cfg!(target_os = "macos") {
        for control in yaar_window::kept_keys() {
            assert!(
                control.key().starts_with("cmd-")
                    || [
                        "ctrl-tab",
                        "ctrl-shift-tab",
                        "shift-pageup",
                        "shift-pagedown"
                    ]
                    .contains(&control.key().as_str()),
                "{} is kept on macOS on the key {}",
                control.title,
                control.key()
            );
        }
    } else {
        assert_eq!(kept.len(), 6, "on Linux the kept keys are six");
    }
}

struct Row {
    key: String,
    bytes: Vec<u8>,
}

fn unhex(text: &str) -> Vec<u8> {
    assert!(text.len().is_multiple_of(2), "odd hex {text:?}");
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).expect("hex"))
        .collect()
}

fn matrix() -> Vec<Row> {
    let rows: Vec<Row> = LEGACY
        .lines()
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .filter(|fields| fields[0] == "legacy")
        .filter(|fields| fields.get(4).is_none_or(|system| *system == SYSTEM))
        .map(|fields| Row {
            key: fields[1].to_owned(),
            bytes: unhex(fields[2]),
        })
        .collect();
    assert!(
        !rows.is_empty(),
        "the matrix has no row of the legacy state"
    );
    rows
}

/// The text follows the last `->` that has any after it, as `ctrl->` is Ctrl with `>`.
fn without_text(key: &str) -> &str {
    key.rfind("->")
        .filter(|at| *at + 2 < key.len())
        .map_or(key, |at| &key[..at])
}

/// GPUI's parser cannot write the key `-` with its text, so the matrix says `minus` and the name is
/// put back.
fn keystroke(key: &str, os: gpui_kit::OperatingSystem) -> Keystroke {
    let mut keystroke = Keystroke::parse_with_os(key, os)
        .unwrap_or_else(|error| panic!("the matrix writes the key {key:?}: {error}"));
    if keystroke.key == "minus" {
        keystroke.key = "-".to_owned();
    }
    keystroke
}

struct Walk {
    window: Composed,
    cx: VisualTestContext,
    recorder: TerminalId,
    file: PathBuf,
    sentinels: u32,
    read: usize,
    differences: Vec<String>,
}

impl Walk {
    fn pane_state(&mut self) -> (Vec<TerminalId>, bool) {
        self.window
            .terminals
            .read_with(&self.cx, |pane, _| (pane.shown(), pane.split()))
    }

    fn in_the_recorder(&mut self) -> bool {
        let (shown, split) = self.pane_state();
        if split || shown != [self.recorder.clone()] {
            return false;
        }
        let view = self
            .window
            .terminals
            .read_with(&self.cx, |pane, _| pane.terminal(&self.recorder))
            .expect("the pane holds the recorder");
        let focus = view.read_with(&self.cx, |view, _| view.focus_handle());
        self.cx.update(|window, _| focus.is_focused(window))
    }

    /// Puts the window back: no split view, the recorder shown and clicked into. Split View and
    /// Previous Terminal are each pressed after a click in the review pane: either may move the
    /// focus into a terminal, where on Linux the next key of the app does nothing.
    fn click_into_the_recorder(&mut self) {
        let terminals = self
            .window
            .terminals
            .read_with(&self.cx, |pane, _| pane.terminals().len());
        let mut presses = 0;
        while self.pane_state().1 {
            self.click_into_the_review_pane();
            self.press(&key_of("Split View"));
            presses += 1;
            assert!(presses <= 2, "Split View does not leave the split view");
        }
        let mut presses = 0;
        while self.pane_state().0 != [self.recorder.clone()] {
            assert!(
                presses <= terminals,
                "Previous Terminal does not come back to the recorder"
            );
            self.click_into_the_review_pane();
            self.press(&key_of("Previous Terminal"));
            presses += 1;
        }
        let at = self.window.point_in("terminal-slot-0", &mut self.cx);
        self.cx.simulate_click(at, gpui_kit::Modifiers::none());
        self.cx.run_until_parked();
    }

    fn click_into_the_review_pane(&mut self) {
        self.window.click_into_the_review_pane(&mut self.cx);
    }

    fn in_the_review_pane(&mut self) -> bool {
        self.window
            .focus_is_in(yaar_review_pane::KEY_CONTEXT, &mut self.cx)
            && !self.window.focus_is_in(PROGRAM_CONTEXT, &mut self.cx)
    }

    fn tidy(&mut self) {
        if self.cx.did_prompt_for_paths() {
            self.cx.simulate_path_prompt_response(|_| None);
        }
        for (selector, what) in [
            ("frame-base-field", "the field for the base"),
            ("frame-worktree-field", "the field for the new branch"),
            ("settings-screen", "the settings screen"),
        ] {
            if self.cx.debug_bounds(selector).is_some() {
                self.cx.simulate_keystrokes("escape");
                wait_until(&mut self.cx, &format!("{what} to close"), |cx| {
                    cx.debug_bounds(selector).is_none()
                });
            }
        }
    }

    /// A key that opens a terminal is waited for until the terminal is in the pane, so that it
    /// does not land in a later step and take the focus from the pane a later key is pressed in.
    fn press(&mut self, key: &str) -> Vec<&'static str> {
        let terminals = self
            .window
            .terminals
            .read_with(&self.cx, |pane, _| pane.terminals().len());
        self.window.ran.borrow_mut().clear();
        self.cx.update(|window, cx| {
            let keystroke = keystroke(key, cx.platform_info().os);
            window.dispatch_keystroke(keystroke, cx);
        });
        self.cx.run_until_parked();
        let ran = self.window.ran.take();
        if ran.contains(&NewTerminal.name()) || ran.contains(&NewAgentTerminal.name()) {
            wait_until(&mut self.cx, "the new terminal in the pane", |cx| {
                self.window
                    .terminals
                    .read_with(cx, |pane, _| pane.terminals().len() > terminals)
            });
        }
        ran
    }

    fn sentinel(&mut self) -> Vec<u8> {
        self.sentinels += 1;
        let text = format!("<s{}>", self.sentinels);
        self.cx.simulate_input(&text);
        let mut file = Vec::new();
        let (path, read) = (self.file.clone(), self.read);
        wait_until(
            &mut self.cx,
            &format!("the sentinel {text} in the file"),
            |_| {
                file = fs::read(&path).unwrap_or_default();
                file.len() >= read + text.len() && file.ends_with(text.as_bytes())
            },
        );
        let gained = file[read..file.len() - text.len()].to_vec();
        self.read = file.len();
        gained
    }

    fn assert_no_difference(&mut self, leg: &str, checked: usize) {
        let differences = std::mem::take(&mut self.differences);
        assert!(
            differences.is_empty(),
            "{} of {checked} keys of {leg} differ on {SYSTEM}:\n{}",
            differences.len(),
            differences.join("\n")
        );
    }
}

fn set_up(cx: &mut TestAppContext) -> Walk {
    let (mut window, mut cx) = open_composed(cx, Vec::new());
    window.open_a_folder(&mut cx);
    // The test platform cannot hide the app, hide the others or minimise (`unimplemented!()`), and
    // closing the window would end the walk: a handler registered after the app's takes them first.
    cx.update(|_, app| {
        app.on_action(|_: &yaar_frame::CloseWindow, _| {});
        app.on_action(|_: &yaar_frame::HideApp, _| {});
        app.on_action(|_: &yaar_frame::HideOtherApps, _| {});
        app.on_action(|_: &yaar_frame::Minimize, _| {});
    });

    window.click_into_the_review_pane(&mut cx);
    cx.simulate_keystrokes(&key_of("New Terminal"));
    wait_until(&mut cx, "the first terminal", |cx| {
        window
            .terminals
            .read_with(cx, |pane, _| !pane.terminals().is_empty())
    });
    let recorder = window
        .terminals
        .read_with(&cx, |pane, _| pane.terminals()[0].clone());
    let file = window.world.home().join("recorded");
    let script = window.world.key_recorder();

    let mut walk = Walk {
        window,
        cx,
        recorder,
        file,
        sentinels: 0,
        read: 0,
        differences: Vec::new(),
    };
    walk.click_into_the_recorder();
    walk.cx
        .simulate_input(&format!("{} {}", script.display(), walk.file.display()));
    walk.cx.simulate_keystrokes("enter");
    let path = walk.file.clone();
    wait_until(&mut walk.cx, "the key recorder to run", |_| path.exists());
    assert_eq!(walk.sentinel(), b"", "the file holds only what was typed");
    walk
}

fn controls_the_matrix_does_not_hold(keep: impl Fn(&Control) -> bool) -> Vec<Control> {
    let held: BTreeSet<String> = matrix()
        .iter()
        .map(|row| without_text(&row.key).to_owned())
        .collect();
    let mut seen = BTreeSet::new();
    yaar_window::controls()
        .into_iter()
        .filter(|control| keep(control))
        .filter(|control| !held.contains(&control.key()))
        .filter(|control| seen.insert(control.key()))
        .collect()
}

#[gpui_kit::test]
fn with_a_terminal_focused_the_kept_keys_run_their_actions_and_no_other_key_does_and_with_another_pane_focused_no_key_reaches_the_program(
    cx: &mut TestAppContext,
) {
    let mut walk = set_up(cx);
    let quit = key_of("Quit");
    // The quit that stops everything would stop this walk's Keeper, so this test leaves it out;
    // the test of the actions presses it.
    let stop = key_of("Quit and Stop Agents");
    let close_terminal = key_of("Close Terminal");
    let new_terminal = key_of("New Terminal");
    let kept = yaar_window::kept_keys();
    let kept_keys: BTreeSet<String> = kept.iter().map(Control::key).collect();

    let mut checked = 0;
    for control in kept
        .iter()
        .filter(|control| control.key() != quit && control.key() != stop)
    {
        if control.key() == close_terminal {
            // Open a spare terminal so closing the focused terminal keeps the recorder running.
            walk.click_into_the_review_pane();
            walk.press(&new_terminal);
            wait_until(&mut walk.cx, "the spare terminal to take focus", |cx| {
                walk.window.focus_is_in(PROGRAM_CONTEXT, cx)
            });
            let before = walk
                .window
                .terminals
                .read_with(&walk.cx, |pane, _| pane.terminals().len());
            let ran = walk.press(&control.key());
            let name = (control.action)().name();
            if ran != [name] {
                walk.differences.push(format!(
                    "{}: the key {} ran {ran:?}, and not {name}",
                    control.title,
                    control.key()
                ));
            }
            wait_until(&mut walk.cx, "the spare terminal to close", |cx| {
                walk.window
                    .terminals
                    .read_with(cx, |pane, _| pane.terminals().len() < before)
            });
            walk.tidy();
            walk.click_into_the_recorder();
            let sent = walk.sentinel();
            if !sent.is_empty() {
                walk.differences.push(format!(
                    "{}: the program received {sent:02x?}",
                    control.title
                ));
            }
            checked += 1;
            continue;
        }
        walk.click_into_the_recorder();
        if !walk.in_the_recorder() {
            walk.differences.push(format!(
                "{}: the focus was not in the recorder before it",
                control.title
            ));
        }
        let ran = walk.press(&control.key());
        let name = (control.action)().name();
        if ran != [name] {
            walk.differences.push(format!(
                "{}: the key {} ran {ran:?}, and not {name}",
                control.title,
                control.key()
            ));
        }
        walk.tidy();
        walk.click_into_the_recorder();
        let sent = walk.sentinel();
        if !sent.is_empty() {
            walk.differences.push(format!(
                "{}: the program received {sent:02x?}",
                control.title
            ));
        }
        checked += 1;
    }
    for title in ["Next Pane", "Previous Pane"] {
        let key = key_of(title);
        walk.click_into_the_recorder();
        walk.press(&key);
        if !walk
            .window
            .focus_is_in(yaar_review_pane::KEY_CONTEXT, &mut walk.cx)
            || walk.window.focus_is_in(PROGRAM_CONTEXT, &mut walk.cx)
        {
            walk.differences
                .push(format!("{title}: the focus is not in the review pane"));
        }
        walk.press(&key);
        if !walk.in_the_recorder() {
            walk.differences
                .push(format!("{title} twice: the focus is not in the recorder"));
        }
        let sent = walk.sentinel();
        if !sent.is_empty() {
            walk.differences
                .push(format!("{title} twice: the program received {sent:02x?}"));
        }
        checked += 1;
    }
    walk.assert_no_difference("kept keys with a terminal focused", checked);

    walk.click_into_the_recorder();
    walk.sentinel();
    let rows = matrix();
    let unheld = controls_the_matrix_does_not_hold(|control| !kept_keys.contains(&control.key()));
    let mut checked = 0;
    let mut presses: Vec<(String, Option<Vec<u8>>)> = rows
        .iter()
        .filter(|row| !kept_keys.contains(without_text(&row.key)))
        .map(|row| (row.key.clone(), Some(row.bytes.clone())))
        .collect();
    presses.extend(unheld.iter().map(|control| (control.key(), None)));
    for (key, wanted) in &presses {
        if !walk.in_the_recorder() {
            walk.differences.push(format!(
                "{key}: the window was not in the recorder before it"
            ));
            walk.click_into_the_recorder();
        }
        let ran = walk.press(key);
        if !ran.is_empty() {
            walk.differences
                .push(format!("{key}: with a terminal focused it ran {ran:?}"));
        }
        walk.tidy();
        if !walk.in_the_recorder() {
            walk.differences
                .push(format!("{key}: the key took the focus out of the recorder"));
            walk.click_into_the_recorder();
        }
        let got = walk.sentinel();
        match wanted {
            Some(bytes) if got != *bytes => walk.differences.push(format!(
                "{key}: the program received {got:02x?}, the matrix says {bytes:02x?}"
            )),
            None if got.is_empty() => walk.differences.push(format!(
                "{key}: a key of the app that is not kept sent nothing"
            )),
            _ => {}
        }
        checked += 1;
    }
    walk.assert_no_difference("the other keys with a terminal focused", checked);

    let mut every: Vec<String> = rows.iter().map(|row| row.key.clone()).collect();
    every.extend(
        controls_the_matrix_does_not_hold(|_| true)
            .iter()
            .map(Control::key),
    );
    let mut seen = BTreeSet::new();
    let mut checked = 0;
    for key in every
        .iter()
        .filter(|key| without_text(key) != quit && without_text(key) != stop)
        .filter(|key| seen.insert(without_text(key).to_owned()))
    {
        let closing = without_text(key) == close_terminal;
        if closing {
            walk.click_into_the_review_pane();
            walk.press(&new_terminal);
        } else {
            walk.click_into_the_recorder();
        }
        walk.click_into_the_review_pane();
        if !walk.in_the_review_pane() {
            walk.differences.push(format!(
                "{key}: the focus was not in the review pane before it"
            ));
        }
        let before = walk
            .window
            .terminals
            .read_with(&walk.cx, |pane, _| pane.terminals().len());
        walk.press(key);
        if closing {
            wait_until(&mut walk.cx, "the spare terminal to close", |cx| {
                walk.window
                    .terminals
                    .read_with(cx, |pane, _| pane.terminals().len() < before)
            });
        }
        walk.tidy();
        checked += 1;
    }
    if !cfg!(target_os = "macos") {
        // On Linux Quit reaches the program when pressed in the terminal, so it is pressed here,
        // where it runs.
        walk.click_into_the_recorder();
        walk.click_into_the_review_pane();
        if !walk.in_the_review_pane() {
            walk.differences.push(format!(
                "{quit}: the focus was not in the review pane before it"
            ));
        }
        let ran = walk.press(&quit);
        if ran != [yaar_window::Quit.name()] {
            walk.differences.push(format!(
                "{quit}: with the review pane focused it ran {ran:?}"
            ));
        }
        walk.tidy();
        checked += 1;
    }
    walk.assert_no_difference("the keys with the review pane focused", checked);
    walk.click_into_the_recorder();
    assert_eq!(
        walk.sentinel(),
        b"",
        "a key reached the program while another pane had the focus"
    );

    // On macOS Quit is a kept key and comes last: the test platform does not end the app.
    if cfg!(target_os = "macos") {
        walk.click_into_the_recorder();
        let ran = walk.press(&quit);
        assert_eq!(ran, [yaar_window::Quit.name()], "what {quit} ran");
        assert_eq!(walk.sentinel(), b"", "{quit} reached the program");
    }
}

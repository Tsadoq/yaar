//! Component test: the keys the app keeps for itself, in the composed Window with the
//! built Keeper beneath it and a real shell in a real terminal.
//!
//! The program that listens is `World::key_recorder`, started by its path typed in the
//! shell of the terminal: it puts its terminal in raw mode and writes down every byte it
//! reads, in a file the test reads from outside the app. What reached the program is what
//! that file holds. A key is followed by a sentinel, text typed into the same terminal
//! whose bytes are known, and what the file holds up to it is what the keys before it
//! sent: a key that sends nothing is shown by the sentinel arriving first. The test waits
//! for facts with a deadline and never for time.
//!
//! What a key ran is read from a keystroke observer on the window, by the name of the
//! action. A key the focused element took itself, as a terminal takes the keys it sends to
//! its program, runs no action and is not reported.

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

/// The bytes a terminal in the legacy state receives for each key, as the terminal pane's
/// own test of them holds them: the expected bytes of TERM-01, which this test reads
/// rather than copies.
const LEGACY: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../terminal-pane/tests/matrix/legacy.tsv"
));

/// The keys of the documented list on the system the test runs on, as the table binds
/// them.
fn documented() -> BTreeSet<String> {
    let mut keys: Vec<&str> = vec![
        "ctrl-tab",
        "ctrl-shift-tab",
        "shift-pageup",
        "shift-pagedown",
    ];
    if cfg!(target_os = "macos") {
        keys.extend([
            // Copy and paste, and every key of the Frame, of the terminal pane and of the
            // Window program that works anywhere: each is a Cmd key.
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
            "cmd-]",
            "cmd-[",
            "cmd-d",
            "cmd-q",
            // The controls of a review attached at an agent's terminal, of slice 6.
            "cmd-alt-v",
            "cmd-alt-x",
            "cmd-alt-a",
            // The launchers and the resume control of the terminal pane, of slice 7.
            "cmd-shift-t",
            "cmd-alt-c",
            "cmd-shift-r",
            // The answers to a question for leave, of slice 7.
            "cmd-alt-e",
            "cmd-alt-shift-e",
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

// ---------------------------------------------------------------------------- the data

/// One row of the legacy state of the matrix that holds on this system.
struct Row {
    /// The key as the matrix writes it, with the text it types after `->` where it types
    /// one.
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

/// A key without the text it types: `alt-a->å` is `alt-a`. The text follows the last
/// `->` that has any after it, as `ctrl->` is Ctrl with `>`.
fn without_text(key: &str) -> &str {
    key.rfind("->")
        .filter(|at| *at + 2 < key.len())
        .map_or(key, |at| &key[..at])
}

/// A key of the matrix as the platform delivers it. GPUI's parser cannot write the key `-`
/// together with its text, so the matrix calls it `minus` and the platform's name for it is
/// put back here.
fn keystroke(key: &str, os: gpui_kit::OperatingSystem) -> Keystroke {
    let mut keystroke = Keystroke::parse_with_os(key, os)
        .unwrap_or_else(|error| panic!("the matrix writes the key {key:?}: {error}"));
    if keystroke.key == "minus" {
        keystroke.key = "-".to_owned();
    }
    keystroke
}

// ------------------------------------------------------------------------------ the walk

/// The test's hold on the window, the recorder and what the recorder has been read up to.
struct Walk {
    window: Composed,
    cx: VisualTestContext,
    recorder: TerminalId,
    /// The file the recorder writes.
    file: PathBuf,
    /// How many sentinels were typed.
    sentinels: u32,
    /// How much of the file was read: up to the end of the last sentinel.
    read: usize,
    /// What went wrong, as one line each, to be shown together at the end of a leg.
    differences: Vec<String>,
}

impl Walk {
    fn pane_state(&mut self) -> (Vec<TerminalId>, bool) {
        self.window
            .terminals
            .read_with(&self.cx, |pane, _| (pane.shown(), pane.split()))
    }

    /// Whether the window is as `click_into_the_recorder` leaves it: the recorder alone on
    /// screen, and the focus in its view.
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

    /// Puts the window back as it was: not in the split view, the recorder the terminal
    /// shown, a click into its view. Each key is pressed after a click in the review
    /// pane, because Split View and Previous Terminal may move the focus into a terminal,
    /// where on Linux the next key of the app does nothing.
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

    /// Whether the focus is in the review pane and in no program.
    fn in_the_review_pane(&mut self) -> bool {
        self.window
            .focus_is_in(yaar_review_pane::KEY_CONTEXT, &mut self.cx)
            && !self.window.focus_is_in(PROGRAM_CONTEXT, &mut self.cx)
    }

    /// Answers the dialog for a folder and closes the field for a base, when a key
    /// opened one.
    fn tidy(&mut self) {
        if self.cx.did_prompt_for_paths() {
            self.cx.simulate_path_prompt_response(|_| None);
        }
        if self.cx.debug_bounds("frame-base-field").is_some() {
            self.cx.simulate_keystrokes("escape");
            wait_until(&mut self.cx, "the field for the base to close", |cx| {
                cx.debug_bounds("frame-base-field").is_none()
            });
        }
    }

    /// Presses a key as the platform delivers it, and gives the names of the actions it
    /// ran. A key that opens a terminal, a shell or an agent's launcher, is waited for until
    /// the terminal is in the pane, so that it does not arrive in the middle of a later step
    /// and take the focus from the pane a later key is pressed in.
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

    /// Types a sentinel into the focused terminal, waits until the file ends with it, and
    /// gives the bytes the file gained before it.
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

    /// Fails at the end of the leg, with every difference of it in one message.
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

/// The set-up: a repository opened, a terminal opened by its key, the key recorder started
/// in it, and its file existing and holding the first sentinel.
fn set_up(cx: &mut TestAppContext) -> Walk {
    let (mut window, mut cx) = open_composed(cx, Vec::new());
    window.open_a_folder(&mut cx);

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
    // The file exists once the shell has started the recorder and the recorder has put its
    // terminal in raw mode: what is typed from here on is recorded as typed.
    let path = walk.file.clone();
    wait_until(&mut walk.cx, "the key recorder to run", |_| path.exists());
    assert_eq!(walk.sentinel(), b"", "the file holds only what was typed");
    walk
}

/// The controls of the Window that `keep` accepts and whose key the matrix does not
/// hold, each key once.
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
    // The quit that stops everything would stop the Keeper of this walk and every
    // program in it: the walk of the actions presses it, last.
    let stop = key_of("Quit and Stop Agents");
    let kept = yaar_window::kept_keys();
    let kept_keys: BTreeSet<String> = kept.iter().map(Control::key).collect();

    // ---- Leg 1: with a terminal focused, each kept key runs its app action.
    let mut checked = 0;
    for control in kept
        .iter()
        .filter(|control| control.key() != quit && control.key() != stop)
    {
        walk.click_into_the_recorder();
        // A key that works anywhere would run its action wherever the focus is, so the
        // focus is checked before the press and not left to the sentinel that follows.
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
        // A kept key never reaches the program: the sentinel is the first thing the file
        // gains after it.
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
    // Next Pane and Previous Pane take the focus out of the terminal and, pressed again,
    // bring it back to the focused terminal, whose view then takes what is typed.
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
        // Neither press sent the program anything: the sentinel is the first thing the
        // file gains.
        let sent = walk.sentinel();
        if !sent.is_empty() {
            walk.differences
                .push(format!("{title} twice: the program received {sent:02x?}"));
        }
        checked += 1;
    }
    walk.assert_no_difference("kept keys with a terminal focused", checked);

    // ---- Leg 2: with a terminal focused, no other key runs an action, and each reaches
    // the program as TERM-01 says for the legacy state, or, for a key of the app that the
    // matrix does not hold, reaches it at all.
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

    // ---- Leg 3: with another pane focused, no key reaches the program.
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
        // A key of this leg may have opened a terminal or moved the pane's focus on to
        // another one: the recorder is made the terminal the pane holds the focus of again
        // before each key, so that a key that leaks reaches the file that is read.
        walk.click_into_the_recorder();
        walk.click_into_the_review_pane();
        if !walk.in_the_review_pane() {
            walk.differences.push(format!(
                "{key}: the focus was not in the review pane before it"
            ));
        }
        walk.press(key);
        walk.tidy();
        checked += 1;
    }
    if !cfg!(target_os = "macos") {
        // The last key of the test on Linux, where Quit was pressed in the terminal and
        // reached the program, so that here it is pressed where it runs.
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

    // ---- Quit comes last of all on macOS, where it is a kept key. The test platform does
    // not end the app, and the test ends here.
    if cfg!(target_os = "macos") {
        walk.click_into_the_recorder();
        let ran = walk.press(&quit);
        assert_eq!(ran, [yaar_window::Quit.name()], "what {quit} ran");
        // The test platform keeps the app running, so a key typed after it shows that Quit,
        // a kept key, sent the program nothing.
        assert_eq!(walk.sentinel(), b"", "{quit} reached the program");
    }
}

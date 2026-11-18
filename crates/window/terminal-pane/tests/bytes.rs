//! Component test: the bytes a program in a terminal receives from the keys and the mouse.
//!
//! The pane runs on GPUI's test platform with the app prepared as the Window program
//! prepares it, and a real pseudo-terminal sits behind the test's Keeper: every `Input`
//! frame the pane sends is written to the master end, and the test reads the slave end,
//! which it holds in raw mode as a program in a terminal does. What it reads is what any
//! program reading its terminal would get; the line discipline in raw mode passes every
//! byte on unchanged, so a difference is the pane's. Nothing runs on the slave: the test
//! is the program, and no process is started.
//!
//! The expected bytes are data, in `tests/matrix/`, one row to a line:
//!
//! ```text
//! state <TAB> key as GPUI writes it <TAB> expected bytes in hex <TAB> source [<TAB> macos|linux]
//! ```
//!
//! A row with a system in its fifth column holds on that system only. The expected bytes
//! are the ones Ghostty sent, recorded in phase 4, and a row that is not Ghostty's says so
//! in its source, with the reason: a departure the plan lists, or a key Ghostty never saw.
//! The keys the app keeps for itself (`departures.tsv`) are not rows of the matrix: they
//! never reach a program. The mouse rows are written from xterm's rules, as nothing of the
//! mouse was recorded.
//!
//! A key is pressed on the test platform as a user's key arrives: through the keystroke
//! interceptors, the key bindings, the view's key listener and, for a key that types text,
//! the input handler. Every key and every mouse action is followed by a sentinel, text
//! whose bytes are known, and what is read up to it is what the key sent: a key that sends
//! nothing is shown by the sentinel arriving first.
//!
//! The Option rows follow the layout of the test platform, which is no layout of macOS:
//! the pane takes Option as characters on macOS, where the matrix holds the text each key
//! types, and Alt as Meta on Linux, where it holds the Meta forms. Each system tests one.

mod common;

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::test::TestEventExt;
use gpui_kit::{
    Keystroke, Modifiers, MouseButton, Pixels, Point, ScrollDelta, ScrollEvent, TestAppContext,
    VisualTestContext, point,
};
use yaar_link::TerminalId;

use common::{DEADLINE, Panes, lines, open_panes_tapping, terminal, wait_until};

const SYSTEM: &str = if cfg!(target_os = "macos") {
    "macos"
} else {
    "linux"
};

const LEGACY: &str = include_str!("matrix/legacy.tsv");
const APPCURSOR: &str = include_str!("matrix/appcursor.tsv");
const KITTY1: &str = include_str!("matrix/kitty1.tsv");
const KITTY3: &str = include_str!("matrix/kitty3.tsv");
const MOUSE: &str = include_str!("matrix/mouse.tsv");
const DEPARTURES: &str = include_str!("matrix/departures.tsv");

// ---------------------------------------------------------------------------- the data

/// One row of the matrix.
struct Row {
    state: String,
    key: String,
    bytes: Vec<u8>,
    source: String,
    system: Option<String>,
}

fn unhex(text: &str) -> Vec<u8> {
    assert!(text.len() % 2 == 0, "odd hex {text:?}");
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).expect("hex"))
        .collect()
}

/// Every row of a file, of every system.
fn all_rows(file: &str) -> Vec<Row> {
    file.lines()
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            assert!(
                (4..=5).contains(&fields.len()),
                "a row has four or five fields: {line:?}"
            );
            Row {
                state: fields[0].to_owned(),
                key: fields[1].to_owned(),
                bytes: unhex(fields[2]),
                source: fields[3].to_owned(),
                system: fields.get(4).map(|system| (*system).to_owned()),
            }
        })
        .collect()
}

/// The rows of one state that hold on the system the test runs on.
fn rows(file: &str, state: &str) -> Vec<Row> {
    let found: Vec<Row> = all_rows(file)
        .into_iter()
        .filter(|row| row.state == state)
        .filter(|row| row.system.as_deref().is_none_or(|system| system == SYSTEM))
        .collect();
    assert!(!found.is_empty(), "no row of the state {state}");
    found
}

// ----------------------------------------------------------------------- the pty bridge

/// A pseudo-terminal: the master end takes what the pane sends, the slave end is what the
/// program reads.
struct Pty {
    master: Arc<File>,
    slave: File,
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    /// macOS has had this since 10.13.4, and the `libc` crate does not declare it there.
    fn ptsname_r(fd: libc::c_int, buf: *mut libc::c_char, buflen: libc::size_t) -> libc::c_int;
}

#[cfg(not(target_os = "macos"))]
use libc::ptsname_r;

fn checked(result: libc::c_int, what: &str) -> libc::c_int {
    assert!(
        result >= 0,
        "{what} failed: {}",
        std::io::Error::last_os_error()
    );
    result
}

impl Pty {
    /// Opens a pair and puts the slave in raw mode, as a program in a terminal does.
    fn open() -> Pty {
        // SAFETY: plain calls of the system on descriptors made here and owned from the
        // moment they exist, and on a buffer as long as the length says.
        unsafe {
            let master = OwnedFd::from_raw_fd(checked(
                libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY),
                "posix_openpt",
            ));
            checked(libc::grantpt(master.as_raw_fd()), "grantpt");
            checked(libc::unlockpt(master.as_raw_fd()), "unlockpt");
            let mut name = [0 as libc::c_char; 128];
            let rc = ptsname_r(master.as_raw_fd(), name.as_mut_ptr(), name.len());
            assert_eq!(rc, 0, "ptsname_r failed");
            let slave = OwnedFd::from_raw_fd(checked(
                libc::open(name.as_ptr(), libc::O_RDWR | libc::O_NOCTTY),
                "open of the slave",
            ));
            let mut attributes = std::mem::zeroed::<libc::termios>();
            checked(
                libc::tcgetattr(slave.as_raw_fd(), &mut attributes),
                "tcgetattr",
            );
            libc::cfmakeraw(&mut attributes);
            attributes.c_cc[libc::VMIN] = 1;
            attributes.c_cc[libc::VTIME] = 0;
            checked(
                libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &attributes),
                "tcsetattr",
            );
            Pty {
                master: Arc::new(File::from(master)),
                slave: File::from(slave),
            }
        }
    }

    /// A function that writes bytes to the master, to hand the test's Keeper as its tap.
    fn tap(&self) -> impl FnMut(&TerminalId, &[u8]) + Send + 'static {
        let master = self.master.clone();
        move |_, bytes| {
            (&*master)
                .write_all(bytes)
                .expect("write to the master of the pty");
        }
    }

    /// Reads the slave until what was read ends with `ending`, and gives what came before
    /// it. Fails at the deadline, saying what did come.
    fn read_until(&self, ending: &[u8]) -> Vec<u8> {
        let deadline = Instant::now() + DEADLINE;
        let mut bytes = Vec::new();
        while !bytes.ends_with(ending) {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "the program received {bytes:?} and not the sentinel {ending:?}"
            );
            let mut waiting = libc::pollfd {
                fd: self.slave.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: one descriptor, and a count of one.
            let ready = unsafe { libc::poll(&mut waiting, 1, left.as_millis() as libc::c_int) };
            if ready <= 0 {
                continue;
            }
            let mut buffer = [0u8; 4096];
            // SAFETY: the buffer is as long as the length says; the descriptor is open.
            let count = unsafe {
                libc::read(
                    self.slave.as_raw_fd(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                )
            };
            assert!(count > 0, "the slave of the pty gave no byte");
            bytes.extend_from_slice(&buffer[..count as usize]);
        }
        bytes.truncate(bytes.len() - ending.len());
        bytes
    }
}

#[test]
fn the_pty_in_raw_mode_passes_every_byte_unchanged() {
    // The test reads what a program gets from the line discipline, so the bridge itself has
    // to add and take away nothing: not a carriage return, a Ctrl-C, an end of file, a
    // newline, a control character of any kind.
    let pty = Pty::open();
    let every: Vec<u8> = (0..=255).collect();
    (&*pty.master).write_all(&every).expect("write");
    assert_eq!(pty.read_until(&[255]), &every[..255]);
}

// ------------------------------------------------------------------------ the test rig

/// A pane with one terminal, focused, on a pty.
struct Rig {
    panes: Panes,
    cx: VisualTestContext,
    id: TerminalId,
    pty: Pty,
    sentinels: usize,
}

impl Rig {
    /// A terminal whose program wrote `state` first, as a program sets what it asks of its
    /// terminal. The wait is for a marker the program writes after it, since the output
    /// travels on a thread of its own.
    fn new(cx: &mut TestAppContext, state: &[u8]) -> Rig {
        let pty = Pty::open();
        let (mut panes, mut cx) = open_panes_tapping(cx, pty.tap());
        let id = terminal('1');
        panes.open_terminal(&id, &mut cx);
        let mut all = state.to_vec();
        all.extend_from_slice(b"\x1b[H\x1b[2Kstate-set");
        panes.keeper.output(&id, &all);
        let pane = panes.pane.clone();
        let watched = id.clone();
        wait_until(&mut cx, "the state on the screen", move |cx| {
            lines(&pane, &watched, cx).first().map(String::as_str) == Some("state-set")
        });
        let mut rig = Rig {
            panes,
            cx,
            id,
            pty,
            sentinels: 0,
        };
        // Whatever the terminal said of itself on the way, and its first size, is behind
        // this: the cells a mouse action aims at are known after it.
        rig.settle();
        rig
    }

    /// Types the next sentinel and reads what the program received before it.
    fn settle(&mut self) -> Vec<u8> {
        self.sentinels += 1;
        let sentinel = format!("<s{}>", self.sentinels);
        self.cx.simulate_input(&sentinel);
        self.pty.read_until(sentinel.as_bytes())
    }

    /// Whether the focus is where a key lands in the terminal.
    fn has_the_focus(&mut self) -> bool {
        self.panes.focused_terminal(&mut self.cx).as_ref() == Some(&self.id)
    }

    /// Puts the focus back into the terminal, when a key took it away.
    fn refocus(&mut self) {
        self.panes.click_slot(0, &mut self.cx);
    }

    /// Presses a key as the platform delivers it, with its text when it types one, and
    /// gives what the program received.
    fn press(&mut self, key: &str) -> Vec<u8> {
        self.cx.update(|window, cx| {
            let keystroke = Keystroke::parse_with_os(key, cx.platform_info().os)
                .unwrap_or_else(|error| panic!("the matrix writes the key {key:?}: {error}"));
            window.dispatch_keystroke(keystroke, cx);
        });
        self.cx.run_until_parked();
        self.settle()
    }

    /// Presses a key and lets it go, and gives what the program received.
    fn press_and_release(&mut self, key: &str) -> Vec<u8> {
        self.cx.update(|window, cx| {
            let keystroke = Keystroke::parse_with_os(key, cx.platform_info().os)
                .unwrap_or_else(|error| panic!("the matrix writes the key {key:?}: {error}"));
            window.dispatch_keystroke(keystroke, cx);
            window.key_up(key, cx);
        });
        self.cx.run_until_parked();
        self.settle()
    }

    /// The middle of a cell of the terminal in the first slot.
    fn cell(&mut self, column: usize, line: usize) -> Point<Pixels> {
        self.panes
            .cell_centre(&self.id, 0, column, line, &mut self.cx)
    }
}

/// Shows the keys of a state that send what the matrix does not say, as one failure with
/// every difference in it.
fn assert_no_difference(state: &str, checked: usize, differences: Vec<String>) {
    assert!(
        differences.is_empty(),
        "{} of {checked} rows of {state} differ on {SYSTEM}:\n{}",
        differences.len(),
        differences.join("\n")
    );
}

/// Walks the rows of a state of the keyboard: the program set `state`, each key of the
/// matrix is pressed, and what the program received is set against its row.
fn walk_keys(cx: &mut TestAppContext, state: &[u8], file: &str, name: &str, release: bool) {
    let mut rig = Rig::new(cx, state);
    let wanted = rows(file, name);
    let mut differences = Vec::new();
    for row in &wanted {
        let got = if release {
            rig.press_and_release(&row.key)
        } else {
            rig.press(&row.key)
        };
        if !rig.has_the_focus() {
            rig.refocus();
            differences.push(format!(
                "{}: the key took the focus out of the terminal",
                row.key
            ));
        }
        if got != row.bytes {
            differences.push(format!(
                "{}: the program received {:02x?}, the matrix says {:02x?} ({})",
                row.key, got, row.bytes, row.source
            ));
        }
    }
    assert_no_difference(name, wanted.len(), differences);
}

// ------------------------------------------------------------------------- the keyboard

#[gpui_kit::test]
fn in_the_legacy_state_every_key_of_the_matrix_sends_the_expected_bytes(cx: &mut TestAppContext) {
    walk_keys(cx, b"", LEGACY, "legacy", false);
}

#[gpui_kit::test]
fn with_application_cursor_keys_every_key_of_the_matrix_sends_the_expected_bytes(
    cx: &mut TestAppContext,
) {
    walk_keys(cx, b"\x1b[?1h", APPCURSOR, "appcursor", false);
}

#[gpui_kit::test]
fn after_a_program_pushed_kitty_flags_5_it_gets_flag_1_forms_for_every_key(
    cx: &mut TestAppContext,
) {
    // The program pushes 5 (disambiguate and alternate keys) and is granted 1.
    walk_keys(cx, b"\x1b[>5u", KITTY1, "kitty1", false);
}

#[gpui_kit::test]
fn after_a_program_pushed_kitty_flags_7_it_gets_the_press_of_every_key_in_flag_1_forms(
    cx: &mut TestAppContext,
) {
    // The program pushes 7 (and so asks for alternate keys) and is granted 1 and 2.
    walk_keys(cx, b"\x1b[>7u", KITTY3, "kitty3", false);
}

#[gpui_kit::test]
fn after_a_program_pushed_kitty_flags_7_the_release_of_a_key_sends_what_the_matrix_says(
    cx: &mut TestAppContext,
) {
    // A key goes down and up: the program gets its press, which the rows above show, and
    // then its release. The matrix holds what the release alone sends, so the press, which
    // the same key shows in the state of its own, is read from it.
    let mut rig = Rig::new(cx, b"\x1b[>7u");
    let presses: HashMap<String, Vec<u8>> = rows(KITTY3, "kitty3")
        .into_iter()
        .map(|row| (row.key, row.bytes))
        .collect();
    let wanted = rows(KITTY3, "kitty3-release");
    let mut differences = Vec::new();
    for row in &wanted {
        let press = presses
            .get(&row.key)
            .unwrap_or_else(|| panic!("a release of {} with no press in the matrix", row.key));
        let mut expected = press.clone();
        expected.extend_from_slice(&row.bytes);
        let got = rig.press_and_release(&row.key);
        if !rig.has_the_focus() {
            rig.refocus();
            differences.push(format!(
                "{}: the key took the focus out of the terminal",
                row.key
            ));
        }
        if got != expected {
            differences.push(format!(
                "{}: the program received {:02x?}, the press and the release say {:02x?} ({})",
                row.key, got, expected, row.source
            ));
        }
    }
    assert_no_difference("kitty3-release", wanted.len(), differences);
}

// ----------------------------------------------------------------------------- the mouse

/// What the program asked of the terminal for each state of the mouse in the matrix.
fn mouse_state(name: &str) -> &'static [u8] {
    match name {
        "off" => b"",
        "1000" => b"\x1b[?1000h",
        "1000-sgr" => b"\x1b[?1000h\x1b[?1006h",
        "1002" => b"\x1b[?1002h",
        "1002-sgr" => b"\x1b[?1002h\x1b[?1006h",
        "1003" => b"\x1b[?1003h",
        "1003-sgr" => b"\x1b[?1003h\x1b[?1006h",
        "alt-scroll" => b"\x1b[?1049h\x1b[?1007h",
        "alt-scroll-appcursor" => b"\x1b[?1049h\x1b[?1007h\x1b[?1h",
        other => panic!("the matrix has a mouse state {other} that the test cannot set"),
    }
}

/// The modifiers of an action's name and the rest of it: `shift-press-left` is Shift and
/// `press-left`.
fn split_modifiers(name: &str) -> (Modifiers, &str) {
    for (prefix, modifiers) in [
        (
            "shift-",
            Modifiers {
                shift: true,
                ..Modifiers::none()
            },
        ),
        (
            "alt-",
            Modifiers {
                alt: true,
                ..Modifiers::none()
            },
        ),
        (
            "ctrl-",
            Modifiers {
                control: true,
                ..Modifiers::none()
            },
        ),
    ] {
        if let Some(rest) = name.strip_prefix(prefix) {
            return (modifiers, rest);
        }
    }
    (Modifiers::none(), name)
}

fn button(name: &str) -> MouseButton {
    match name {
        "left" => MouseButton::Left,
        "middle" => MouseButton::Middle,
        "right" => MouseButton::Right,
        other => panic!("no mouse button {other}"),
    }
}

impl Rig {
    /// Does one action of the matrix at the cell in column 3 and line 1 and gives what the
    /// program received for the action alone. What an action needs before it (a press
    /// before a release or a drag) and what it leaves behind (a button still down) is done
    /// and read apart, so that each row is one action's bytes.
    fn act(&mut self, name: &str) -> Vec<u8> {
        let (modifiers, action) = split_modifiers(name);
        let none = Modifiers::none();
        let aimed = self.cell(3, 1);
        let before = self.cell(2, 1);
        let parts: Vec<&str> = action.split('-').collect();
        match parts.as_slice() {
            ["press", which] => {
                self.cx.simulate_mouse_down(aimed, button(which), modifiers);
                let got = self.settle();
                self.cx.simulate_mouse_up(aimed, button(which), none);
                self.settle();
                got
            }
            ["release", which] => {
                self.cx.simulate_mouse_down(aimed, button(which), none);
                self.settle();
                self.cx.simulate_mouse_up(aimed, button(which), modifiers);
                self.settle()
            }
            ["drag", which] => {
                self.cx.simulate_mouse_down(before, button(which), none);
                self.settle();
                self.cx
                    .simulate_mouse_move(aimed, Some(button(which)), modifiers);
                let got = self.settle();
                self.cx.simulate_mouse_up(aimed, button(which), none);
                self.settle();
                got
            }
            ["move"] => {
                // Out of the cell first, so that the move the row is about is one into it.
                self.cx.simulate_mouse_move(before, None, none);
                self.settle();
                self.cx.simulate_mouse_move(aimed, None, modifiers);
                self.settle()
            }
            ["wheel", direction] => {
                let lines = if *direction == "up" { 1. } else { -1. };
                self.cx.simulate_event(ScrollEvent {
                    position: aimed,
                    delta: ScrollDelta::Lines(point(0., lines)),
                    modifiers,
                    ..ScrollEvent::default()
                });
                self.settle()
            }
            other => panic!("the matrix has the mouse action {other:?}"),
        }
    }
}

fn walk_mouse(cx: &mut TestAppContext, state: &str) {
    let mut rig = Rig::new(cx, mouse_state(state));
    let wanted = rows(MOUSE, state);
    let mut differences = Vec::new();
    for row in &wanted {
        let got = rig.act(&row.key);
        if got != row.bytes {
            differences.push(format!(
                "{}: the program received {:02x?}, the matrix says {:02x?} ({})",
                row.key, got, row.bytes, row.source
            ));
        }
    }
    assert_no_difference(state, wanted.len(), differences);
}

#[gpui_kit::test]
fn a_program_that_asked_for_no_mouse_reports_is_sent_none(cx: &mut TestAppContext) {
    walk_mouse(cx, "off");
}

#[gpui_kit::test]
fn mouse_reports_of_mode_1000_say_presses_releases_and_the_wheel_in_the_old_form(
    cx: &mut TestAppContext,
) {
    walk_mouse(cx, "1000");
}

#[gpui_kit::test]
fn mouse_reports_of_mode_1000_say_presses_releases_and_the_wheel_in_the_sgr_form(
    cx: &mut TestAppContext,
) {
    walk_mouse(cx, "1000-sgr");
}

#[gpui_kit::test]
fn mouse_reports_of_mode_1002_add_a_drag_in_the_old_form(cx: &mut TestAppContext) {
    walk_mouse(cx, "1002");
}

#[gpui_kit::test]
fn mouse_reports_of_mode_1002_add_a_drag_in_the_sgr_form(cx: &mut TestAppContext) {
    walk_mouse(cx, "1002-sgr");
}

#[gpui_kit::test]
fn mouse_reports_of_mode_1003_add_every_move_in_the_old_form(cx: &mut TestAppContext) {
    walk_mouse(cx, "1003");
}

#[gpui_kit::test]
fn mouse_reports_of_mode_1003_add_every_move_in_the_sgr_form(cx: &mut TestAppContext) {
    walk_mouse(cx, "1003-sgr");
}

#[gpui_kit::test]
fn on_the_alternate_screen_with_alternate_scroll_the_wheel_is_the_cursor_keys(
    cx: &mut TestAppContext,
) {
    walk_mouse(cx, "alt-scroll");
}

#[gpui_kit::test]
fn on_the_alternate_screen_with_alternate_scroll_and_application_cursor_keys_the_wheel_is_the_application_form(
    cx: &mut TestAppContext,
) {
    walk_mouse(cx, "alt-scroll-appcursor");
}

// ------------------------------------------------------------------------- the data itself

#[test]
fn the_matrix_has_a_source_for_every_row_and_one_row_for_a_key_in_a_state_on_a_system() {
    let mut seen = HashSet::new();
    for (file, states) in [
        (LEGACY, &["legacy"][..]),
        (APPCURSOR, &["appcursor"]),
        (KITTY1, &["kitty1"]),
        (KITTY3, &["kitty3", "kitty3-release"]),
        (
            MOUSE,
            &[
                "off",
                "1000",
                "1000-sgr",
                "1002",
                "1002-sgr",
                "1003",
                "1003-sgr",
                "alt-scroll",
                "alt-scroll-appcursor",
            ],
        ),
    ] {
        let every = all_rows(file);
        for state in states {
            assert!(
                every.iter().any(|row| row.state == *state),
                "no row of {state}"
            );
        }
        for row in every {
            assert!(
                states.contains(&row.state.as_str()),
                "the state {} is in the wrong file",
                row.state
            );
            assert!(
                !row.source.is_empty() && !row.key.is_empty(),
                "a row of {} with no key or no source",
                row.state
            );
            assert!(
                matches!(row.system.as_deref(), None | Some("macos" | "linux")),
                "{}: a system that is none",
                row.key
            );
            for system in ["macos", "linux"] {
                if row.system.as_deref().is_none_or(|own| own == system) {
                    assert!(
                        seen.insert((row.state.clone(), row.key.clone(), system)),
                        "{} of {} is twice in the matrix on {system}",
                        row.key,
                        row.state
                    );
                }
            }
        }
    }
}

#[test]
fn a_key_that_is_left_out_of_the_matrix_as_a_departure_is_no_row_of_it() {
    let departures: Vec<Vec<&str>> = DEPARTURES
        .lines()
        .map(|line| line.split('\t').collect())
        .collect();
    assert!(!departures.is_empty());
    for fields in &departures {
        assert_eq!(
            fields.len(),
            4,
            "a departure has state, key, system, reason"
        );
        assert!(!fields[3].is_empty(), "{}: no reason", fields[1]);
        let (state, key, system) = (fields[0], fields[1], fields[2]);
        let file = match state {
            "legacy" => LEGACY,
            "appcursor" => APPCURSOR,
            "kitty1" => KITTY1,
            "kitty3" => KITTY3,
            other => panic!("a departure in the state {other}"),
        };
        for row in all_rows(file) {
            let same_system =
                system.is_empty() || row.system.as_deref().is_none_or(|own| own == system);
            assert!(
                !(row.key == key && same_system && row.state.starts_with(state)),
                "{key} is a departure of {state} and a row of it too"
            );
        }
    }
}

/// The modifiers of a key as the matrix writes it, and its name: `alt-shift-a->Å` is Alt
/// and Shift, and `a`.
fn modifiers_and_name(key: &str) -> (Vec<&str>, &str) {
    // The text follows the last `->` that has any after it: `ctrl->` is Ctrl with `>`.
    let key = key
        .rfind("->")
        .filter(|at| *at + 2 < key.len())
        .map_or(key, |at| &key[..at]);
    let mut modifiers = Vec::new();
    let mut rest = key;
    while rest.len() > 1 {
        let Some(prefix) = ["ctrl-", "alt-", "shift-", "cmd-"]
            .into_iter()
            .find(|prefix| rest.starts_with(prefix))
        else {
            break;
        };
        modifiers.push(&prefix[..prefix.len() - 1]);
        rest = &rest[prefix.len()..];
    }
    (modifiers, rest)
}

#[test]
fn the_matrix_holds_each_key_of_its_groups_with_each_modifier_but_those_a_departure_leaves_out() {
    // A guard on the data's reach: a file cut short fails here and not in silence. A key
    // that is left out is named in `departures.tsv`, with its reason.
    let departures: Vec<(String, String)> = DEPARTURES
        .lines()
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .filter(|fields| fields[2].is_empty() || fields[2] == SYSTEM)
        .map(|fields| {
            (
                fields[0].to_owned(),
                modifiers_and_name(fields[1]).1.to_owned(),
            )
        })
        .collect();
    let digits_and_signs = [
        ("0", ")"),
        ("1", "!"),
        ("2", "@"),
        ("3", "#"),
        ("4", "$"),
        ("5", "%"),
        ("6", "^"),
        ("7", "&"),
        ("8", "*"),
        ("9", "("),
        ("`", "~"),
        ("-", "_"),
        ("=", "+"),
        ("[", "{"),
        ("]", "}"),
        ("\\", "|"),
        (";", ":"),
        ("'", "\""),
        (",", "<"),
        (".", ">"),
        ("/", "?"),
    ];
    for (file, state) in [
        (LEGACY, "legacy"),
        (APPCURSOR, "appcursor"),
        (KITTY1, "kitty1"),
        (KITTY3, "kitty3"),
    ] {
        // How many rows hold the key of each name, apart from the Cmd rows.
        let mut count: HashMap<String, usize> = HashMap::new();
        let mut cmd: HashSet<String> = HashSet::new();
        for row in rows(file, state) {
            let (modifiers, name) = modifiers_and_name(&row.key);
            if modifiers.contains(&"cmd") {
                cmd.insert(name.to_owned());
            } else {
                *count.entry(name.to_owned()).or_default() += 1;
            }
        }
        let left_out = |names: &[&str]| {
            departures
                .iter()
                .filter(|(of, name)| of == state && names.contains(&name.as_str()))
                .count()
        };
        let mut check = |names: &[&str], full: usize| {
            let found: usize = names
                .iter()
                .map(|name| count.get(*name).copied().unwrap_or(0))
                .sum();
            assert_eq!(
                found,
                full - left_out(names),
                "{state}: rows of {names:?} on {SYSTEM}"
            );
        };
        // None, Shift, Ctrl, Alt, Ctrl-Shift, Alt-Shift.
        for letter in 'a'..='z' {
            check(&[&letter.to_string()], 6);
        }
        for (plain, shifted) in digits_and_signs {
            check(&[plain, shifted], 6);
        }
        for name in ["enter", "tab", "backspace", "escape", "space", "delete"] {
            check(&[name], 6);
        }
        // None, Shift, Alt and Ctrl.
        for name in [
            "up", "down", "left", "right", "home", "end", "pageup", "pagedown",
        ] {
            check(&[name], 4);
        }
        for number in 1..=12 {
            check(&[&format!("f{number}")], 1);
        }
        // Cmd with the cursor keys on macOS, and the line edit the app adds there.
        let expected: HashSet<String> = if cfg!(target_os = "macos") {
            [
                "up",
                "down",
                "left",
                "right",
                "home",
                "end",
                "pageup",
                "pagedown",
                "backspace",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect()
        } else {
            HashSet::new()
        };
        assert_eq!(cmd, expected, "{state}: the rows with Cmd on {SYSTEM}");
    }
}

//! Component test (TERM-08): the keys the terminal pane has on macOS for its history and
//! its screen: Cmd-K empties the history and the screen above the prompt's line, Cmd-A
//! selects everything, and the Cmd scroll keys move to the top, the bottom and one line.
//! None of them sends a byte to the program. On Linux these six actions have no key, and
//! only Ctrl-Shift-C and Ctrl-Shift-V are kept in a terminal, so the file is macOS's.
//!
//! The pane runs on GPUI's test platform with the app prepared as the Window program
//! prepares it, and the test plays the Keeper. The program writes 200 numbered lines and
//! a prompt with the cursor after it; the keys are pressed as the table has them
//! (`key_of`), and read back through the screen the view shows and the clipboard. That no
//! byte reached the program is read after a barrier: a sentinel typed after the keys comes
//! through the same path, and what arrives before it is what the keys sent.

#![cfg(target_os = "macos")]

mod common;

use gpui_kit::{ClipboardItem, TestAppContext, VisualTestContext};
use yaar_link::TerminalId;
use yaar_terminal_pane::{
    Clear, Copy, ScrollLineDown, ScrollLineUp, ScrollToBottom, ScrollToTop, SelectAll,
};

use common::{Panes, key_of, lines, open_panes, terminal, wait_drawn, wait_until};

/// The lines the program writes before its prompt.
const WRITTEN: usize = 200;

/// The prompt, which the cursor follows.
const PROMPT: &str = "prompt$";

/// What the program writes: numbered lines, then the prompt with the cursor after it.
fn output() -> Vec<u8> {
    let mut bytes = Vec::new();
    for number in 1..=WRITTEN {
        bytes.extend(format!("line {number:03}\r\n").as_bytes());
    }
    bytes.extend(format!("{PROMPT} ").as_bytes());
    bytes
}

/// The text of the line `number` of the output.
fn line(number: usize) -> String {
    format!("line {number:03}")
}

/// A pane with one terminal whose program wrote [`output`].
struct Screen {
    panes: Panes,
    cx: VisualTestContext,
    id: TerminalId,
    sentinels: usize,
}

impl Screen {
    fn new(cx: &mut TestAppContext) -> Screen {
        let (mut panes, mut cx) = open_panes(cx);
        let id = terminal('1');
        panes.open_terminal(&id, &mut cx);
        wait_drawn(&mut cx, "the terminal to have the focus", |cx| {
            panes.focused_terminal(cx).as_ref() == Some(&id)
        });
        panes.keeper.output(&id, &output());
        let (pane, watched) = (panes.pane.clone(), id.clone());
        wait_until(&mut cx, "the prompt on the screen", |cx| {
            lines(&pane, &watched, cx).iter().any(|line| line == PROMPT)
        });
        Screen {
            panes,
            cx,
            id,
            sentinels: 0,
        }
    }

    /// The lines of the screen, as the view shows them.
    fn lines(&mut self) -> Vec<String> {
        lines(&self.panes.pane, &self.id, &self.cx)
    }

    /// How far the view is scrolled back into the history.
    fn scrolled_back(&mut self) -> usize {
        let view = self
            .panes
            .pane
            .read_with(&self.cx, |pane, _| pane.terminal(&self.id))
            .expect("the terminal of the pane");
        view.read_with(&self.cx, |view, _| view.scrolled_back())
    }

    /// Presses the key the table has for the action.
    fn press<A: gpui_kit::Action + Default>(&mut self) {
        self.cx.simulate_keystrokes(key_of::<A>());
    }

    /// Types the next sentinel and reads what the program received before it: what the
    /// keys pressed since the last one sent.
    fn received(&mut self) -> Vec<u8> {
        self.sentinels += 1;
        let sentinel = format!("<s{}>", self.sentinels);
        self.cx.simulate_input(&sentinel);
        self.panes
            .keeper
            .input_until(&self.id, sentinel.as_bytes(), &mut self.cx)
    }

    /// The text on the clipboard.
    fn clipboard(&self) -> Option<String> {
        self.cx.read_from_clipboard().and_then(|item| item.text())
    }
}

#[gpui_kit::test]
fn cmd_k_empties_the_history_and_the_screen_above_the_prompt_and_sends_nothing(
    cx: &mut TestAppContext,
) {
    let mut screen = Screen::new(cx);
    let rows = screen.lines().len();
    // The control half: there is history to empty, and the prompt is on the last line.
    screen.press::<ScrollToTop>();
    assert_eq!(screen.scrolled_back(), WRITTEN + 1 - rows);
    screen.press::<ScrollToBottom>();
    assert_eq!(screen.lines().last().map(String::as_str), Some(PROMPT));

    screen.press::<Clear>();

    // The prompt's line is the first, and nothing is below it.
    let shown = screen.lines();
    assert_eq!(shown.len(), rows);
    assert_eq!(shown[0], PROMPT);
    assert!(shown[1..].iter().all(String::is_empty), "{shown:?}");
    // No history is left to scroll to: the top is the screen.
    screen.press::<ScrollToTop>();
    assert_eq!(screen.scrolled_back(), 0);
    assert_eq!(screen.lines(), shown);
    // The program received no byte from either key.
    assert_eq!(screen.received(), b"");
}

#[gpui_kit::test]
fn cmd_k_in_a_view_scrolled_into_the_history_brings_the_prompt_back(cx: &mut TestAppContext) {
    let mut screen = Screen::new(cx);
    screen.press::<ScrollToTop>();
    assert_ne!(screen.scrolled_back(), 0);

    screen.press::<Clear>();

    assert_eq!(screen.scrolled_back(), 0);
    assert_eq!(screen.lines()[0], PROMPT);
    assert_eq!(screen.received(), b"");
}

#[gpui_kit::test]
fn what_the_program_writes_after_cmd_k_lands_under_the_prompt(cx: &mut TestAppContext) {
    let mut screen = Screen::new(cx);
    screen.press::<Clear>();
    screen.panes.keeper.output(&screen.id, b"after");
    let (pane, id) = (screen.panes.pane.clone(), screen.id.clone());
    wait_until(&mut screen.cx, "the program's next output", |cx| {
        lines(&pane, &id, cx)[0] == format!("{PROMPT} after")
    });
}

#[gpui_kit::test]
fn cmd_a_selects_the_whole_history_and_screen_and_copy_puts_it_on_the_clipboard(
    cx: &mut TestAppContext,
) {
    let mut screen = Screen::new(cx);
    screen
        .cx
        .write_to_clipboard(ClipboardItem::new_string("kept".to_owned()));
    // Control: with nothing selected, copying leaves the clipboard as it is.
    screen.press::<Copy>();
    assert_eq!(screen.clipboard().as_deref(), Some("kept"));

    screen.press::<SelectAll>();
    screen.press::<Copy>();

    let mut expected: Vec<String> = (1..=WRITTEN).map(line).collect();
    expected.push(PROMPT.to_owned());
    let copied = screen.clipboard().expect("text on the clipboard");
    assert_eq!(copied.lines().collect::<Vec<_>>(), expected);
    assert_eq!(screen.received(), b"");
}

#[gpui_kit::test]
fn the_cmd_scroll_keys_move_to_the_top_the_bottom_and_one_line_and_send_nothing(
    cx: &mut TestAppContext,
) {
    let mut screen = Screen::new(cx);
    let rows = screen.lines().len();
    let history = WRITTEN + 1 - rows;
    // At the newest line, the first line shown is the one after the history.
    assert_eq!(screen.scrolled_back(), 0);
    assert_eq!(screen.lines()[0], line(history + 1));

    screen.press::<ScrollToTop>();
    assert_eq!(screen.scrolled_back(), history);
    assert_eq!(screen.lines()[0], line(1));

    // One line down: one line on, towards the newest.
    screen.press::<ScrollLineDown>();
    assert_eq!(screen.scrolled_back(), history - 1);
    assert_eq!(screen.lines()[0], line(2));

    // One line up: back to the top, and no further.
    screen.press::<ScrollLineUp>();
    assert_eq!(screen.lines()[0], line(1));
    screen.press::<ScrollLineUp>();
    assert_eq!(screen.scrolled_back(), history);
    assert_eq!(screen.lines()[0], line(1));

    screen.press::<ScrollToBottom>();
    assert_eq!(screen.scrolled_back(), 0);
    assert_eq!(screen.lines().last().map(String::as_str), Some(PROMPT));

    // One line up from the newest, and down again.
    screen.press::<ScrollLineUp>();
    assert_eq!(screen.scrolled_back(), 1);
    assert_eq!(screen.lines()[0], line(history));
    screen.press::<ScrollLineDown>();
    assert_eq!(screen.scrolled_back(), 0);
    // And no further than the newest line.
    screen.press::<ScrollLineDown>();
    assert_eq!(screen.scrolled_back(), 0);

    assert_eq!(screen.received(), b"");
}

#[gpui_kit::test]
fn the_cmd_keys_of_the_history_are_the_terminals_and_do_nothing_elsewhere(cx: &mut TestAppContext) {
    let mut screen = Screen::new(cx);
    let before = screen.lines();
    screen.panes.click_elsewhere(&mut screen.cx);
    screen.press::<ScrollToTop>();
    screen.press::<Clear>();
    screen.press::<SelectAll>();
    screen.press::<Copy>();
    // The test platform runs each key to its end before the call returns, so nothing of
    // these is still on its way; a click back into the terminal gives it the focus again.
    screen.panes.click_slot(0, &mut screen.cx);
    assert_eq!(screen.scrolled_back(), 0);
    assert_eq!(screen.lines(), before);
    assert_eq!(screen.received(), b"");
}

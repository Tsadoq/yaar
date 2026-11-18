//! Component test: dead keys and an input method, driven through the terminal's input
//! handler on GPUI's test platform. The handler is driven as the system's input method
//! drives it, by marking text while it composes and replacing it with the result; the
//! system's own input methods are tried by hand.

mod common;

use gpui_kit::test::TestInput;
use gpui_kit::{ElementInputHandler, InputHandler, TestAppContext, VisualTestContext};
use yaar_link::TerminalId;
use yaar_terminal_pane::TerminalView;

use common::{Panes, drawn, open_panes, terminal};

/// A pane with one terminal, whose view has the focus, and the view.
fn one_terminal(
    cx: &mut TestAppContext,
) -> (
    Panes,
    VisualTestContext,
    TerminalId,
    gpui_kit::Entity<TerminalView>,
) {
    let (mut panes, mut cx) = open_panes(cx);
    let id = terminal('1');
    panes.open_terminal(&id, &mut cx);
    let view = panes
        .pane
        .read_with(&cx, |pane, _| pane.terminal(&id))
        .expect("the terminal of the pane");
    (panes, cx, id, view)
}

/// Marks `text` as an input method does while it composes.
fn compose(text: &str, view: &gpui_kit::Entity<TerminalView>, cx: &mut VisualTestContext) {
    let bounds = drawn("terminal-slot-0", cx).expect("the slot is drawn");
    let handler = ElementInputHandler::new(bounds, view.clone());
    cx.update(|window, cx| TestInput::new(handler, window).compose_text(None, text, None, cx));
}

/// Commits `text` as an input method does when the composition ends.
fn commit(text: &str, view: &gpui_kit::Entity<TerminalView>, cx: &mut VisualTestContext) {
    let bounds = drawn("terminal-slot-0", cx).expect("the slot is drawn");
    let handler = ElementInputHandler::new(bounds, view.clone());
    cx.update(|window, cx| TestInput::new(handler, window).commit_text(text, cx));
}

fn marked(view: &gpui_kit::Entity<TerminalView>, cx: &VisualTestContext) -> Option<String> {
    view.read_with(cx, |view, _| view.marked_text())
}

/// The dead keys of macOS on the ABC layout, and on the U.S. layout with Option set to
/// characters (on the U.S. layout the app's default makes Option Meta, which composes
/// nothing): the accent the first key marks, and what the second key makes of it. The
/// system gives the same texts on both layouts, so one list shows both. The last three
/// are the ways a dead key ends without the letter it waits for: a letter that does not
/// combine commits the accent and the letter, Space the accent alone, and Backspace the
/// accent too, where Ghostty drops the Backspace.
const DEAD_KEYS: [(&str, &str, &str); 10] = [
    // Option-e, e
    ("\u{b4}", "e", "é"),
    // Option-e, Shift-e
    ("\u{b4}", "E", "É"),
    // Option-u, o
    ("\u{a8}", "o", "ö"),
    // Option-u, u
    ("\u{a8}", "u", "ü"),
    // Option-n, n
    ("\u{2dc}", "n", "ñ"),
    // Option-`, a
    ("`", "a", "à"),
    // Option-i, e
    ("\u{2c6}", "e", "ê"),
    // Option-e, x
    ("\u{b4}", "x", "\u{b4}x"),
    // Option-u, Space
    ("\u{a8}", "space", "\u{a8}"),
    // Option-n, Backspace
    ("\u{2dc}", "backspace", "\u{2dc}"),
];

#[gpui_kit::test]
fn each_dead_key_pair_gives_the_composed_character_and_an_input_method_marks_and_commits(
    cx: &mut TestAppContext,
) {
    let (panes, mut cx, id, view) = one_terminal(cx);

    for (accent, second, result) in DEAD_KEYS {
        // The first key: the accent is marked, drawn at the cursor, and sent to nobody.
        compose(accent, &view, &mut cx);
        assert_eq!(
            marked(&view, &cx).as_deref(),
            Some(accent),
            "after the dead key of {accent:?}"
        );
        assert!(
            drawn("terminal-marked-text", &mut cx).is_some(),
            "{accent:?} is marked and not drawn"
        );
        // The second key: the composed text is committed, whole, as it is. Anything the
        // dead key sent would be the first frame of input and not this one.
        commit(result, &view, &mut cx);
        assert_eq!(
            panes.keeper.input(&mut cx),
            (id.clone(), result.as_bytes().to_vec()),
            "{accent:?} then {second:?} should give {result:?}"
        );
        assert_eq!(marked(&view, &cx), None);
        assert!(drawn("terminal-marked-text", &mut cx).is_none());
    }
    cx.run_until_parked();
    assert!(panes.keeper.no_input_waits());
}

#[gpui_kit::test]
fn an_input_method_shows_what_it_composes_and_commits_the_word(cx: &mut TestAppContext) {
    let (panes, mut cx, id, view) = one_terminal(cx);

    // Pinyin, as the system composes `nihao`: each state of the composition is shown,
    // and none of it is sent.
    for composing in ["n", "ni", "ni h", "ni ha", "ni hao"] {
        compose(composing, &view, &mut cx);
        assert_eq!(marked(&view, &cx).as_deref(), Some(composing));
        assert!(
            drawn("terminal-marked-text", &mut cx).is_some(),
            "{composing:?} is not drawn"
        );
    }
    commit("你好", &view, &mut cx);
    let (from, bytes) = panes.keeper.input(&mut cx);
    assert_eq!(from, id);
    assert_eq!(bytes, [0xe4, 0xbd, 0xa0, 0xe5, 0xa5, 0xbd]);
    assert_eq!(marked(&view, &cx), None);
    assert!(drawn("terminal-marked-text", &mut cx).is_none());

    // Nothing of the composition was sent before the word: the word is all there is.
    cx.run_until_parked();
    assert!(panes.keeper.no_input_waits());
}

#[gpui_kit::test]
fn a_composition_that_ends_in_nothing_sends_nothing(cx: &mut TestAppContext) {
    let (panes, mut cx, id, view) = one_terminal(cx);

    // An empty text cancels what was marked.
    compose("´", &view, &mut cx);
    compose("", &view, &mut cx);
    assert_eq!(marked(&view, &cx), None);
    assert!(drawn("terminal-marked-text", &mut cx).is_none());

    // Unmarking ends it too, and sends nothing: a text sent after is the first frame.
    compose("¨", &view, &mut cx);
    let bounds = drawn("terminal-slot-0", &mut cx).expect("the slot is drawn");
    let handler = ElementInputHandler::new(bounds, view.clone());
    cx.update(|window, cx| TestInput::new(handler, window).unmark_text(cx));
    assert_eq!(marked(&view, &cx), None);

    commit("x", &view, &mut cx);
    assert_eq!(panes.keeper.input(&mut cx), (id, b"x".to_vec()));
}

#[gpui_kit::test]
fn the_ranges_of_the_handler_count_utf16_units_of_the_marked_text(cx: &mut TestAppContext) {
    let (_panes, mut cx, _id, view) = one_terminal(cx);
    let bounds = drawn("terminal-slot-0", &mut cx).expect("the slot is drawn");
    let mut handler = ElementInputHandler::new(bounds, view.clone());

    // No document: with nothing marked there is nothing to ask for.
    let (marked_range, text, selection) = cx.update(|window, cx| {
        let mut adjusted = None;
        (
            handler.marked_text_range(window, cx),
            handler.text_for_range(0..4, &mut adjusted, window, cx),
            handler.selected_text_range(false, window, cx),
        )
    });
    assert_eq!((marked_range, text), (None, None));
    assert_eq!(selection.map(|selection| selection.range), Some(0..0));

    // Marked text of three characters and four units: the last one takes two.
    compose("你好🙂", &view, &mut cx);
    let (marked_range, text, adjusted, selection) = cx.update(|window, cx| {
        let mut adjusted = None;
        let text = handler.text_for_range(1..4, &mut adjusted, window, cx);
        (
            handler.marked_text_range(window, cx),
            text,
            adjusted,
            handler.selected_text_range(false, window, cx),
        )
    });
    assert_eq!(marked_range, Some(0..4));
    assert_eq!(text.as_deref(), Some("好🙂"));
    assert_eq!(adjusted, Some(1..4));
    assert_eq!(selection.map(|selection| selection.range), Some(4..4));

    // A range past the end is cut to it.
    let (text, adjusted) = cx.update(|window, cx| {
        let mut adjusted = None;
        let text = handler.text_for_range(2..40, &mut adjusted, window, cx);
        (text, adjusted)
    });
    assert_eq!(text.as_deref(), Some("🙂"));
    assert_eq!(adjusted, Some(2..4));
}

#[gpui_kit::test]
fn the_candidate_window_is_put_at_the_cursor(cx: &mut TestAppContext) {
    let (panes, mut cx, id, view) = one_terminal(cx);
    let slot = drawn("terminal-slot-0", &mut cx).expect("the slot is drawn");
    let mut handler = ElementInputHandler::new(slot, view.clone());
    let at_the_cursor = |handler: &mut ElementInputHandler<TerminalView>,
                         cx: &mut VisualTestContext| {
        cx.update(|window, cx| handler.bounds_for_range(0..0, window, cx))
            .expect("the cursor is shown")
    };

    let first = at_the_cursor(&mut handler, &mut cx);
    assert_eq!(first.origin, slot.origin);

    // Three characters on, the cursor is three cells to the right; the cell is as wide
    // as the bounds say.
    panes.keeper.output(&id, b"abc");
    common::wait_until(&mut cx, "the output", |cx| {
        common::lines(&panes.pane, &id, cx)
            .first()
            .map(String::as_str)
            == Some("abc")
    });
    let third = at_the_cursor(&mut handler, &mut cx);
    assert_eq!(third.size, first.size);
    assert_eq!(third.origin.y, first.origin.y);
    let moved = third.origin.x - first.origin.x;
    assert!(
        (moved - first.size.width * 3.).abs() < gpui_kit::px(0.01),
        "{moved:?}"
    );
}

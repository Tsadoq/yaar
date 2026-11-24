mod common;

use gpui_kit::test::TestInput;
use gpui_kit::{ElementInputHandler, InputHandler, TestAppContext, VisualTestContext};
use yaar_link::TerminalId;
use yaar_terminal_pane::TerminalView;

use common::{Panes, drawn, open_panes, terminal};

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

fn compose(text: &str, view: &gpui_kit::Entity<TerminalView>, cx: &mut VisualTestContext) {
    let bounds = drawn("terminal-slot-0", cx).expect("the slot is drawn");
    let handler = ElementInputHandler::new(bounds, view.clone());
    cx.update(|window, cx| TestInput::new(handler, window).compose_text(None, text, None, cx));
}

fn commit(text: &str, view: &gpui_kit::Entity<TerminalView>, cx: &mut VisualTestContext) {
    let bounds = drawn("terminal-slot-0", cx).expect("the slot is drawn");
    let handler = ElementInputHandler::new(bounds, view.clone());
    cx.update(|window, cx| TestInput::new(handler, window).commit_text(text, cx));
}

fn marked(view: &gpui_kit::Entity<TerminalView>, cx: &VisualTestContext) -> Option<String> {
    view.read_with(cx, |view, _| view.marked_text())
}

/// Dead keys of macOS on ABC, and on U.S. with Option as characters: (accent, second key,
/// result). In the last three no letter combines: a plain letter commits accent and letter,
/// Space and Backspace the accent alone.
const DEAD_KEYS: [(&str, &str, &str); 10] = [
    ("\u{b4}", "e", "é"),
    ("\u{b4}", "E", "É"),
    ("\u{a8}", "o", "ö"),
    ("\u{a8}", "u", "ü"),
    ("\u{2dc}", "n", "ñ"),
    ("`", "a", "à"),
    ("\u{2c6}", "e", "ê"),
    ("\u{b4}", "x", "\u{b4}x"),
    ("\u{a8}", "space", "\u{a8}"),
    ("\u{2dc}", "backspace", "\u{2dc}"),
];

#[gpui_kit::test]
fn each_dead_key_pair_gives_the_composed_character_and_an_input_method_marks_and_commits(
    cx: &mut TestAppContext,
) {
    let (panes, mut cx, id, view) = one_terminal(cx);

    for (accent, second, result) in DEAD_KEYS {
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

    cx.run_until_parked();
    assert!(panes.keeper.no_input_waits());
}

#[gpui_kit::test]
fn a_composition_that_ends_in_nothing_sends_nothing(cx: &mut TestAppContext) {
    let (panes, mut cx, id, view) = one_terminal(cx);

    compose("´", &view, &mut cx);
    compose("", &view, &mut cx);
    assert_eq!(marked(&view, &cx), None);
    assert!(drawn("terminal-marked-text", &mut cx).is_none());

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

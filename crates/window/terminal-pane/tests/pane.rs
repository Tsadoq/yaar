mod common;

use std::time::Duration;

use gpui_kit::{
    ClipboardItem, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, MouseButton, ScrollDelta,
    ScrollEvent, TestAppContext, VisualTestContext, point, px, size,
};
use yaar_link::{Ending, Fact, Reply, Request, Setting, TerminalId, TerminalPlace};
use yaar_terminal_pane::{
    Copy, FIRST_SIZE, FRAMES_AFTER_A_KEY, NewTerminal, NextTerminal, Paste, PreviousTerminal,
    ScrollPageUp, ToggleSplit,
};

use common::{
    Panes, drawn, key_of, lines, open_panes, refused, terminal, wait_until, workspace_id,
};

fn has_the_focus(panes: &Panes, id: &TerminalId, cx: &mut VisualTestContext) -> bool {
    let view = panes
        .pane
        .read_with(cx, |pane, _| pane.terminal(id))
        .expect("a terminal of the pane");
    let focus = view.read_with(cx, |view, _| view.focus_handle());
    cx.update(|window, _| focus.is_focused(window))
}

fn shown(panes: &Panes, cx: &mut VisualTestContext) -> Vec<TerminalId> {
    panes.pane.read_with(cx, |pane, _| pane.shown())
}

fn focused(panes: &Panes, cx: &mut VisualTestContext) -> Option<TerminalId> {
    panes.pane.read_with(cx, |pane, _| pane.focused())
}

#[gpui_kit::test]
fn the_pane_holds_several_terminals_at_once_and_switches_between_them(cx: &mut TestAppContext) {
    let (mut panes, mut cx) = open_panes(cx);
    let (t1, t2, t3) = (terminal('1'), terminal('2'), terminal('3'));

    for _ in 0..3 {
        panes.press_in_the_app::<NewTerminal>(&mut cx);
    }
    let asked: Vec<_> = (0..3).map(|_| panes.keeper.request(&mut cx)).collect();
    for (_, request) in &asked {
        let Request::OpenTerminal { workspace, .. } = request else {
            panic!("{request:?} is no question for a terminal");
        };
        assert_eq!(*workspace, workspace_id('1'));
    }
    for (n, (id, terminal)) in [(asked[1].0, &t1), (asked[2].0, &t2), (asked[0].0, &t3)]
        .into_iter()
        .enumerate()
    {
        panes.keeper.reply(
            id,
            Ok(yaar_link::Reply::TerminalOpened {
                terminal: terminal.clone(),
            }),
        );
        let pane = panes.pane.clone();
        wait_until(&mut cx, "the terminal in the pane", move |cx| {
            pane.read_with(cx, |pane, _| pane.terminals().len() == n + 1)
        });
        if n == 0 {
            panes.keeper.output(&t1, b"one$");
        }
    }
    panes.keeper.output(&t3, b"three$");
    panes.keeper.output(&t2, b"two$");
    wait_until(&mut cx, "the output of every terminal", |cx| {
        [(&t1, "one$"), (&t2, "two$"), (&t3, "three$")]
            .into_iter()
            .all(|(id, text)| lines(&panes.pane, id, cx).first().map(String::as_str) == Some(text))
    });

    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.terminals()),
        [t1.clone(), t2.clone(), t3.clone()]
    );
    for id in [&t1, &t2, &t3] {
        assert_eq!(
            panes.pane.read_with(&cx, |pane, _| pane.workspace(id)),
            Some(workspace_id('1'))
        );
    }
    assert_eq!(lines(&panes.pane, &t1, &cx)[0], "one$");
    assert_eq!(lines(&panes.pane, &t2, &cx)[0], "two$");
    assert_eq!(lines(&panes.pane, &t3, &cx)[0], "three$");

    assert_eq!(shown(&panes, &mut cx), vec![t3.clone()]);
    assert_eq!(focused(&panes, &mut cx), Some(t3.clone()));
    assert!(has_the_focus(&panes, &t3, &mut cx));

    let step = |panes: &mut Panes, cx: &mut VisualTestContext, key: &str, now: &TerminalId| {
        panes.click_elsewhere(cx);
        cx.simulate_keystrokes(key);
        assert_eq!(shown(panes, cx), vec![now.clone()], "after {key}");
        assert_eq!(focused(panes, cx), Some(now.clone()), "after {key}");
        assert!(has_the_focus(panes, now, cx), "after {key}");
    };
    step(&mut panes, &mut cx, key_of::<NextTerminal>(), &t1);
    assert_eq!(lines(&panes.pane, &t1, &cx)[0], "one$");
    step(&mut panes, &mut cx, key_of::<PreviousTerminal>(), &t3);
    assert_eq!(lines(&panes.pane, &t3, &cx)[0], "three$");
    step(&mut panes, &mut cx, key_of::<PreviousTerminal>(), &t2);
    assert_eq!(lines(&panes.pane, &t2, &cx)[0], "two$");
    step(&mut panes, &mut cx, key_of::<NextTerminal>(), &t3);
    step(&mut panes, &mut cx, key_of::<NextTerminal>(), &t1);
    step(&mut panes, &mut cx, key_of::<NextTerminal>(), &t2);
}

#[gpui_kit::test]
fn the_split_view_shows_two_terminals_at_once_and_keys_go_to_the_focused_one(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_panes(cx);
    let (t1, t2) = (terminal('1'), terminal('2'));
    panes.open_terminal(&t1, &mut cx);
    panes.open_terminal(&t2, &mut cx);
    assert!(!panes.pane.read_with(&cx, |pane, _| pane.split()));
    assert!(drawn("terminal-slot-1", &mut cx).is_none());

    panes.press_in_the_app::<ToggleSplit>(&mut cx);
    assert!(panes.pane.read_with(&cx, |pane, _| pane.split()));
    let two = shown(&panes, &mut cx);
    assert_eq!(two, [t2.clone(), t1.clone()]);
    let (first, second) = (
        drawn("terminal-slot-0", &mut cx).expect("the first slot is drawn"),
        drawn("terminal-slot-1", &mut cx).expect("the second slot is drawn"),
    );
    assert!(
        first.bottom() <= second.top(),
        "in a side pane the split slots are stacked: {first:?} and {second:?}"
    );

    panes.models.settings.update(&mut cx, |settings, cx| {
        settings.set(Setting::TerminalPlace(TerminalPlace::Below), cx);
    });
    cx.run_until_parked();
    let (below_first, below_second) = (
        drawn("terminal-slot-0", &mut cx).expect("the first slot is drawn below"),
        drawn("terminal-slot-1", &mut cx).expect("the second slot is drawn below"),
    );
    assert!(
        below_first.right() <= below_second.left(),
        "in a bottom pane the split slots are side by side: {below_first:?} and {below_second:?}"
    );
    panes.models.settings.update(&mut cx, |settings, cx| {
        settings.set(Setting::TerminalPlace(TerminalPlace::Right), cx);
    });
    cx.run_until_parked();

    panes.click_slot(1, &mut cx);
    assert_eq!(focused(&panes, &mut cx), Some(two[1].clone()));
    cx.simulate_input("q");
    assert_eq!(panes.keeper.input(&mut cx), (two[1].clone(), b"q".to_vec()));

    panes.click_slot(0, &mut cx);
    assert_eq!(focused(&panes, &mut cx), Some(two[0].clone()));
    cx.simulate_input("w");
    assert_eq!(panes.keeper.input(&mut cx), (two[0].clone(), b"w".to_vec()));

    cx.simulate_keystrokes("escape");
    assert_eq!(
        panes.keeper.input(&mut cx),
        (two[0].clone(), b"\x1b".to_vec())
    );
    panes.click_slot(1, &mut cx);
    cx.simulate_keystrokes("escape");
    assert_eq!(
        panes.keeper.input(&mut cx),
        (two[1].clone(), b"\x1b".to_vec())
    );

    cx.run_until_parked();
    assert!(panes.keeper.no_input_waits());
}

fn frames_asked(cx: &mut VisualTestContext) -> usize {
    let asked = cx.update(|window, cx| window.simulate_next_frame(cx));
    cx.run_until_parked();
    asked
}

fn frames_keep_coming(steps: u32, from: u32, cx: &mut VisualTestContext) {
    for step in 1..=steps {
        cx.executor().advance_clock(Duration::from_millis(100));
        assert!(
            frames_asked(cx) >= 1,
            "no frame was asked for {} ms after the key",
            (from + step) * 100
        );
    }
}

/// Moves the clock to `edge`, where the frame drawn still asks for the next, then 2 ms on, where
/// it asks for none. A count is of the last draw's frames, so the pending frame is drawn first.
fn frames_end_at(from: u64, edge: u64, cx: &mut VisualTestContext, what: &str) {
    cx.executor()
        .advance_clock(Duration::from_millis(edge - from));
    frames_asked(cx);
    assert!(
        frames_asked(cx) >= 1,
        "frames stopped before {what} was over"
    );
    cx.executor().advance_clock(Duration::from_millis(2));
    frames_asked(cx);
    assert_eq!(frames_asked(cx), 0, "frames go on after {what}");
}

#[gpui_kit::test]
fn after_each_key_the_pane_keeps_asking_for_frames_for_one_second(cx: &mut TestAppContext) {
    assert_eq!(FRAMES_AFTER_A_KEY, Duration::from_secs(1));
    let (mut panes, mut cx) = open_panes(cx);
    let one = terminal('1');
    panes.open_terminal(&one, &mut cx);
    assert!(has_the_focus(&panes, &one, &mut cx));
    // The clock does not move, so a pane that asks for frames for ever fails here instead of
    // hanging.
    let mut drawn = 0;
    while frames_asked(&mut cx) > 0 {
        drawn += 1;
        assert!(
            drawn < 20,
            "the window never settled: it still asks for frames"
        );
    }

    cx.simulate_keystrokes("a");
    frames_keep_coming(9, 0, &mut cx);
    frames_end_at(900, 999, &mut cx, "one second");

    cx.simulate_keystrokes("a");
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.simulate_keystrokes(key_of::<ScrollPageUp>());
    frames_keep_coming(9, 5, &mut cx);
    frames_end_at(1_400, 1_499, &mut cx, "1,500 ms");
}

fn wait_for_first_line(panes: &Panes, id: &TerminalId, text: &str, cx: &mut VisualTestContext) {
    wait_until(cx, &format!("{text:?} on the screen"), |cx| {
        panes
            .pane
            .read_with(cx, |pane, _| pane.terminal(id))
            .is_some()
            && lines(&panes.pane, id, cx).first().map(String::as_str) == Some(text)
    });
}

/// Writes `bytes`, then `marker` on the top line, and waits for the marker: the output travels
/// on its own thread, so a state it sets (a mouse mode) holds only once later output is on screen.
fn program_writes(
    panes: &Panes,
    id: &TerminalId,
    bytes: &[u8],
    marker: &str,
    cx: &mut VisualTestContext,
) {
    let mut all = bytes.to_vec();
    all.extend_from_slice(format!("\x1b[H\x1b[2K{marker}").as_bytes());
    panes.keeper.output(id, &all);
    wait_for_first_line(panes, id, marker, cx);
}

fn one_terminal(cx: &mut TestAppContext) -> (Panes, VisualTestContext, TerminalId) {
    let (mut panes, mut cx) = open_panes(cx);
    let one = terminal('1');
    panes.open_terminal(&one, &mut cx);
    (panes, cx, one)
}

#[gpui_kit::test]
fn a_terminal_that_the_keeper_refuses_is_not_added_and_the_reason_is_shown(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_panes(cx);
    panes.press_in_the_app::<NewTerminal>(&mut cx);
    let (id, request) = panes.keeper.request(&mut cx);
    assert!(matches!(request, Request::OpenTerminal { .. }));
    panes.keeper.reply(id, Err(refused("no shell today")));
    wait_until(&mut cx, "the notice", |cx| {
        panes.pane.read_with(cx, |pane, _| pane.notice().is_some())
    });
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.notice()),
        Some("no shell today".to_owned())
    );
    assert!(
        panes
            .pane
            .read_with(&cx, |pane, _| pane.terminals().is_empty())
    );
    assert!(shown(&panes, &mut cx).is_empty());
}

#[gpui_kit::test]
fn what_the_program_wrote_before_the_answer_was_read_is_not_lost(cx: &mut TestAppContext) {
    let (mut panes, mut cx) = open_panes(cx);
    let one = terminal('1');
    panes.press_in_the_app::<NewTerminal>(&mut cx);
    let (id, _) = panes.keeper.request(&mut cx);
    panes.keeper.output(&one, b"prompt$ ");
    panes.keeper.reply(
        id,
        Ok(Reply::TerminalOpened {
            terminal: one.clone(),
        }),
    );
    wait_for_first_line(&panes, &one, "prompt$", &mut cx);
}

#[gpui_kit::test]
fn a_second_press_before_the_first_answer_loses_neither_terminal(cx: &mut TestAppContext) {
    let (mut panes, mut cx) = open_panes(cx);
    for _ in 0..2 {
        panes.press_in_the_app::<NewTerminal>(&mut cx);
    }
    panes.keeper.open(&terminal('1'), &mut cx);
    panes.keeper.open(&terminal('2'), &mut cx);
    wait_until(&mut cx, "both terminals", |cx| {
        panes
            .pane
            .read_with(cx, |pane, _| pane.terminals().len() == 2)
    });
    assert_eq!(shown(&panes, &mut cx), [terminal('2')]);
}

#[gpui_kit::test]
fn the_pane_hands_the_focus_on_to_the_focused_terminal(cx: &mut TestAppContext) {
    let (mut panes, mut cx) = open_panes(cx);
    cx.update(|window, cx| panes.pane_focus.focus(window, cx));
    cx.run_until_parked();
    assert!(cx.update(|window, _| panes.pane_focus.is_focused(window)));

    let (one, two) = (terminal('1'), terminal('2'));
    panes.open_terminal(&one, &mut cx);
    panes.open_terminal(&two, &mut cx);
    panes.click_elsewhere(&mut cx);
    assert!(!has_the_focus(&panes, &two, &mut cx));
    cx.update(|window, cx| panes.pane_focus.focus(window, cx));
    cx.run_until_parked();
    assert!(has_the_focus(&panes, &two, &mut cx));
    cx.simulate_input("k");
    assert_eq!(panes.keeper.input(&mut cx), (two, b"k".to_vec()));
}

#[gpui_kit::test]
fn the_split_view_does_nothing_with_fewer_than_two_terminals(cx: &mut TestAppContext) {
    let (mut panes, mut cx, one) = one_terminal(cx);
    panes.press_in_the_app::<ToggleSplit>(&mut cx);
    assert!(!panes.pane.read_with(&cx, |pane, _| pane.split()));
    assert_eq!(shown(&panes, &mut cx), [one]);
    panes.open_terminal(&terminal('2'), &mut cx);
    panes.press_in_the_app::<ToggleSplit>(&mut cx);
    assert!(panes.pane.read_with(&cx, |pane, _| pane.split()));
    panes.press_in_the_app::<ToggleSplit>(&mut cx);
    assert!(!panes.pane.read_with(&cx, |pane, _| pane.split()));
    assert_eq!(shown(&panes, &mut cx).len(), 1);
}

#[gpui_kit::test]
fn in_the_split_view_the_next_terminal_goes_to_the_other_slot_when_it_is_shown_there(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_panes(cx);
    let ids = [terminal('1'), terminal('2'), terminal('3')];
    for id in &ids {
        panes.open_terminal(id, &mut cx);
    }
    panes.press_in_the_app::<ToggleSplit>(&mut cx);
    assert_eq!(shown(&panes, &mut cx), [ids[2].clone(), ids[0].clone()]);
    panes.press_in_the_app::<NextTerminal>(&mut cx);
    assert_eq!(shown(&panes, &mut cx), [ids[2].clone(), ids[0].clone()]);
    assert_eq!(focused(&panes, &mut cx), Some(ids[0].clone()));
    panes.press_in_the_app::<NextTerminal>(&mut cx);
    assert_eq!(shown(&panes, &mut cx), [ids[2].clone(), ids[1].clone()]);
    assert_eq!(focused(&panes, &mut cx), Some(ids[1].clone()));
    assert!(has_the_focus(&panes, &ids[1], &mut cx));
}

#[gpui_kit::test]
fn a_terminal_that_is_not_shown_keeps_taking_the_output_of_its_program(cx: &mut TestAppContext) {
    let (mut panes, mut cx) = open_panes(cx);
    let (one, two) = (terminal('1'), terminal('2'));
    panes.open_terminal(&one, &mut cx);
    panes.open_terminal(&two, &mut cx);
    assert_eq!(shown(&panes, &mut cx), [two]);
    panes.keeper.output(&one, b"while you were away");
    wait_for_first_line(&panes, &one, "while you were away", &mut cx);
}

#[gpui_kit::test]
fn the_window_title_follows_the_title_of_the_focused_terminal(cx: &mut TestAppContext) {
    let (mut panes, mut cx, one) = one_terminal(cx);
    assert_eq!(cx.window_title().as_deref(), Some("YAAR"));

    panes.keeper.output(&one, b"\x1b]0;hello\x07");
    wait_until(&mut cx, "the title", |cx| {
        let view = panes
            .pane
            .read_with(cx, |pane, _| pane.terminal(&one))
            .expect("the terminal");
        view.read_with(cx, |view, _| view.title()) == Some("hello".to_owned())
    });
    assert_eq!(cx.window_title().as_deref(), Some("hello - YAAR"));

    panes.open_terminal(&terminal('2'), &mut cx);
    assert_eq!(cx.window_title().as_deref(), Some("YAAR"));
    panes.press_in_the_app::<PreviousTerminal>(&mut cx);
    assert_eq!(cx.window_title().as_deref(), Some("hello - YAAR"));

    panes.keeper.output(&one, b"\x1b]0;\x07");
    wait_until(&mut cx, "the title is gone", |cx| {
        let view = panes
            .pane
            .read_with(cx, |pane, _| pane.terminal(&one))
            .expect("the terminal");
        view.read_with(cx, |view, _| view.title())
            .is_none_or(|title| title.is_empty())
    });
    assert_eq!(cx.window_title().as_deref(), Some("YAAR"));
}

#[gpui_kit::test]
fn a_terminal_whose_program_ended_stays_with_its_last_screen_and_says_how(cx: &mut TestAppContext) {
    let (mut panes, mut cx) = open_panes(cx);
    let (one, two) = (terminal('1'), terminal('2'));
    panes.open_terminal(&one, &mut cx);
    panes.open_terminal(&two, &mut cx);
    panes.press_in_the_app::<PreviousTerminal>(&mut cx);
    assert_eq!(focused(&panes, &mut cx), Some(one.clone()));
    panes.keeper.output(&one, b"bye");
    wait_for_first_line(&panes, &one, "bye", &mut cx);
    assert!(drawn("terminal-ended", &mut cx).is_none());

    panes.keeper.tell(Fact::TerminalEnded {
        terminal: one.clone(),
        ending: Ending::Exited(3),
    });
    let view = panes
        .pane
        .read_with(&cx, |pane, _| pane.terminal(&one))
        .expect("the terminal");
    wait_until(&mut cx, "the ending", |cx| {
        view.read_with(cx, |view, _| view.ending()) == Some(Ending::Exited(3))
    });
    assert!(drawn("terminal-ended", &mut cx).is_some());
    assert_eq!(lines(&panes.pane, &one, &cx)[0], "bye");
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.terminals()),
        [one.clone(), two.clone()]
    );

    // What is typed in the ended terminal is not sent. The absence is read up to a sentinel typed
    // in the other terminal, since input travels on threads the test does not wait for.
    cx.simulate_input("x");
    cx.simulate_keystrokes("escape");
    panes.press_in_the_app::<NextTerminal>(&mut cx);
    assert_eq!(focused(&panes, &mut cx), Some(two.clone()));
    cx.simulate_input("s");
    assert_eq!(panes.keeper.input(&mut cx), (two, b"s".to_vec()));
}

#[gpui_kit::test]
fn the_keeper_is_told_the_size_of_the_grid_when_it_changes_and_only_then(cx: &mut TestAppContext) {
    let (mut panes, mut cx, one) = one_terminal(cx);
    wait_until(&mut cx, "the first fit", |_| {
        panes.keeper.resizes(&one).len() == 1
    });
    let fitted = panes.grid(&one);
    assert_ne!(fitted, FIRST_SIZE);

    cx.simulate_input("x");
    assert_eq!(panes.keeper.input(&mut cx).1, b"x");
    for _ in 0..3 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }
    assert_eq!(panes.keeper.resizes(&one), [fitted]);

    cx.simulate_resize(size(px(700.), px(400.)));
    cx.run_until_parked();
    wait_until(&mut cx, "the second fit", |_| {
        panes.keeper.resizes(&one).len() == 2
    });
    let smaller = panes.grid(&one);
    assert!(smaller.columns < fitted.columns && smaller.rows < fitted.rows);
    cx.run_until_parked();
    assert_eq!(panes.keeper.resizes(&one).len(), 2);

    panes.press_in_the_app::<NewTerminal>(&mut cx);
    let (_, asked) = panes.keeper.open(&terminal('2'), &mut cx);
    assert_eq!(asked, smaller);
}

#[gpui_kit::test]
fn a_key_reaches_the_program_once_in_the_form_the_encoder_gives(cx: &mut TestAppContext) {
    let (panes, mut cx, one) = one_terminal(cx);
    for (key, bytes) in [
        ("h", &b"h"[..]),
        ("ctrl-c", &[0x03]),
        ("tab", &[0x09]),
        ("enter", &[0x0d]),
        ("escape", &[0x1b]),
        ("up", b"\x1b[A"),
    ] {
        cx.simulate_keystrokes(key);
        assert_eq!(
            panes.keeper.input(&mut cx),
            (one.clone(), bytes.to_vec()),
            "the key {key}"
        );
    }
    program_writes(&panes, &one, b"\x1b[?1h", "cursor-keys", &mut cx);
    cx.simulate_keystrokes("up");
    assert_eq!(panes.keeper.input(&mut cx).1, b"\x1bOA");
    cx.simulate_input("z");
    assert_eq!(panes.keeper.input(&mut cx).1, b"z");
    assert!(panes.keeper.no_input_waits());
}

#[gpui_kit::test]
fn a_held_letter_repeats_as_text_and_is_not_left_to_the_systems_accent_popup(
    cx: &mut TestAppContext,
) {
    let (panes, mut cx, one) = one_terminal(cx);
    // The first press of a letter is typed text for the input handler, so the key listener sends
    // nothing for it; each repeat of the held key is sent as its text.
    let press = |held: bool| KeyDownEvent {
        keystroke: Keystroke {
            key: "e".to_owned(),
            key_char: Some("e".to_owned()),
            modifiers: Modifiers::none(),
        },
        is_held: held,
        prefer_character_input: false,
    };
    cx.simulate_event(press(false));
    cx.simulate_event(press(true));
    cx.simulate_event(press(true));
    cx.simulate_event(KeyUpEvent {
        keystroke: press(false).keystroke,
    });
    assert_eq!(panes.keeper.input(&mut cx), (one.clone(), b"e".to_vec()));
    assert_eq!(panes.keeper.input(&mut cx), (one.clone(), b"e".to_vec()));
    cx.simulate_input("z");
    assert_eq!(panes.keeper.input(&mut cx), (one, b"z".to_vec()));
}

#[gpui_kit::test]
fn the_terminal_answers_the_questions_of_its_program(cx: &mut TestAppContext) {
    let (panes, mut cx, one) = one_terminal(cx);
    panes.keeper.output(&one, b"\x1b[c");
    let (from, answer) = panes.keeper.input(&mut cx);
    assert_eq!(from, one);
    assert!(
        answer.starts_with(b"\x1b[?") && answer.ends_with(b"c"),
        "{answer:?}"
    );
}

fn fill_with_lines(panes: &Panes, id: &TerminalId, cx: &mut VisualTestContext) -> usize {
    let text: String = (0..200).map(|n| format!("line-{n}\r\n")).collect();
    panes.keeper.output(id, text.as_bytes());
    wait_until(cx, "the lines", |cx| {
        lines(&panes.pane, id, cx)
            .iter()
            .any(|line| line == "line-199")
    });
    top_line(panes, id, cx)
}

fn top_line(panes: &Panes, id: &TerminalId, cx: &mut VisualTestContext) -> usize {
    let top = lines(&panes.pane, id, cx)[0].clone();
    top.strip_prefix("line-")
        .unwrap_or_else(|| panic!("the first line is {top:?}"))
        .parse()
        .expect("a number")
}

#[gpui_kit::test]
fn the_wheel_and_the_scroll_keys_move_through_the_history_and_typing_comes_back(
    cx: &mut TestAppContext,
) {
    let (panes, mut cx, one) = one_terminal(cx);
    let newest = fill_with_lines(&panes, &one, &mut cx);
    assert!(newest > 100, "the screen holds the end of the output");
    let over = panes.cell_centre(&one, 0, 1, 1, &mut cx);
    cx.simulate_mouse_move(over, None, Modifiers::none());

    cx.simulate_event(ScrollEvent {
        position: over,
        delta: ScrollDelta::Lines(point(0., 1.)),
        ..ScrollEvent::default()
    });
    cx.run_until_parked();
    assert_eq!(top_line(&panes, &one, &mut cx), newest - 3);
    cx.simulate_event(ScrollEvent {
        position: over,
        delta: ScrollDelta::Lines(point(0., -1.)),
        ..ScrollEvent::default()
    });
    cx.run_until_parked();
    assert_eq!(top_line(&panes, &one, &mut cx), newest);

    cx.simulate_keystrokes(key_of::<ScrollPageUp>());
    let paged = top_line(&panes, &one, &mut cx);
    assert!(
        paged < newest - 3,
        "a page is more than a step: {paged} and {newest}"
    );
    assert!(panes.keeper.no_input_waits());

    cx.simulate_input("x");
    assert_eq!(panes.keeper.input(&mut cx).1, b"x");
    assert_eq!(top_line(&panes, &one, &mut cx), newest);
}

#[gpui_kit::test]
fn on_the_alternate_screen_a_step_of_the_wheel_is_an_arrow_key_for_the_program(
    cx: &mut TestAppContext,
) {
    let (panes, mut cx, one) = one_terminal(cx);
    program_writes(&panes, &one, b"\x1b[?1049h", "full-screen", &mut cx);
    let over = panes.cell_centre(&one, 0, 1, 1, &mut cx);
    cx.simulate_mouse_move(over, None, Modifiers::none());
    let step = |lines: f32| ScrollEvent {
        position: over,
        delta: ScrollDelta::Lines(point(0., lines)),
        ..ScrollEvent::default()
    };
    cx.simulate_event(step(1.));
    assert_eq!(
        panes.keeper.input(&mut cx),
        (one.clone(), b"\x1b[A".to_vec())
    );
    cx.simulate_event(step(-2.));
    assert_eq!(
        panes.keeper.input(&mut cx),
        (one.clone(), b"\x1b[B\x1b[B".to_vec())
    );
    program_writes(&panes, &one, b"\x1b[?1h", "cursor-keys", &mut cx);
    cx.simulate_event(step(1.));
    assert_eq!(panes.keeper.input(&mut cx), (one, b"\x1bOA".to_vec()));
}

#[gpui_kit::test]
fn a_drag_selects_text_and_copy_puts_the_selection_on_the_clipboard(cx: &mut TestAppContext) {
    let (panes, mut cx, one) = one_terminal(cx);
    panes.keeper.output(&one, b"hello world");
    wait_for_first_line(&panes, &one, "hello world", &mut cx);

    cx.write_to_clipboard(ClipboardItem::new_string("kept".to_owned()));
    cx.simulate_keystrokes(key_of::<Copy>());
    assert_eq!(clipboard(&cx).as_deref(), Some("kept"));

    let from = panes.cell_centre(&one, 0, 0, 0, &mut cx);
    let to = panes.cell_centre(&one, 0, 4, 0, &mut cx);
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    cx.simulate_keystrokes(key_of::<Copy>());
    assert_eq!(clipboard(&cx).as_deref(), Some("hello"));

    cx.write_to_clipboard(ClipboardItem::new_string("kept".to_owned()));
    cx.simulate_click(from, Modifiers::none());
    cx.simulate_keystrokes(key_of::<Copy>());
    assert_eq!(clipboard(&cx).as_deref(), Some("kept"));
    assert!(panes.keeper.no_input_waits());
}

fn clipboard(cx: &VisualTestContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|item| item.text())
}

#[gpui_kit::test]
fn paste_sends_the_clipboard_as_a_paste_and_an_empty_clipboard_sends_nothing(
    cx: &mut TestAppContext,
) {
    let (panes, mut cx, one) = one_terminal(cx);

    cx.simulate_keystrokes(key_of::<Paste>());
    cx.simulate_input("s");
    assert_eq!(panes.keeper.input(&mut cx), (one.clone(), b"s".to_vec()));

    cx.write_to_clipboard(ClipboardItem::new_string("a\nb".to_owned()));
    cx.simulate_keystrokes(key_of::<Paste>());
    assert_eq!(panes.keeper.input(&mut cx), (one.clone(), b"a\rb".to_vec()));

    program_writes(&panes, &one, b"\x1b[?2004h", "bracketed", &mut cx);
    cx.simulate_keystrokes(key_of::<Paste>());
    assert_eq!(
        panes.keeper.input(&mut cx),
        (one, b"\x1b[200~a\rb\x1b[201~".to_vec())
    );
}

#[gpui_kit::test]
fn a_program_that_asked_for_the_mouse_is_told_of_it_and_moves_in_one_cell_are_told_once(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx, one) = one_terminal(cx);
    program_writes(&panes, &one, b"\x1b[?1003h\x1b[?1006h", "mouse-on", &mut cx);

    let a = panes.cell_centre(&one, 0, 3, 2, &mut cx);
    let b = panes.cell_centre(&one, 0, 4, 2, &mut cx);
    cx.simulate_mouse_move(a, None, Modifiers::none());
    cx.simulate_mouse_move(a + point(px(1.), px(1.)), None, Modifiers::none());
    cx.simulate_mouse_move(b, None, Modifiers::none());
    cx.simulate_mouse_down(b, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(b, MouseButton::Left, Modifiers::none());
    cx.simulate_input("z");
    let sent = panes.keeper.input_until(&one, b"z", &mut cx);
    assert_eq!(
        String::from_utf8(sent).expect("text"),
        "\x1b[<35;4;3M\x1b[<35;5;3M\x1b[<0;5;3M\x1b[<0;5;3m"
    );
}

#[gpui_kit::test]
fn a_program_that_asked_for_focus_reports_is_told_when_the_terminal_gains_and_loses_the_focus(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx, one) = one_terminal(cx);
    program_writes(&panes, &one, b"\x1b[?1004h", "focus-on", &mut cx);

    panes.click_elsewhere(&mut cx);
    assert_eq!(
        panes.keeper.input(&mut cx),
        (one.clone(), b"\x1b[O".to_vec())
    );
    panes.click_slot(0, &mut cx);
    assert_eq!(panes.keeper.input(&mut cx), (one, b"\x1b[I".to_vec()));
}

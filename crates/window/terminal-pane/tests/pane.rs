//! Component test: the terminal pane on GPUI's test platform, with no window on any screen.
//! Keys, clicks and the wheel go in; the pane is read back through the model it shows.
//! The test plays the Keeper on the other end of a socket pair: it answers what the pane
//! asks and reads what the pane sends a program.

mod common;

use std::time::Duration;

use gpui_kit::{
    ClipboardItem, Modifiers, MouseButton, ScrollDelta, ScrollEvent, TestAppContext,
    VisualTestContext, point, px,
};
use yaar_link::{Ending, Fact, Request};
use yaar_terminal_pane::{
    FIRST_SIZE, FRAMES_AFTER_A_KEY, NewTerminal, NextTerminal, Paste, PreviousTerminal,
    ScrollPageUp, ToggleSplit,
};

use common::{
    Panes, click_in, drawn, key_of, lines, open_panes, refused, terminal, wait_until, workspace_id,
};

/// Whether the focus is in the view of this terminal.
fn has_the_focus(panes: &Panes, id: &yaar_link::TerminalId, cx: &mut VisualTestContext) -> bool {
    let view = panes
        .pane
        .read_with(cx, |pane, _| pane.terminal(id))
        .expect("a terminal of the pane");
    let focus = view.read_with(cx, |view, _| view.focus_handle());
    cx.update(|window, _| focus.is_focused(window))
}

/// What the pane shows, as the model says it: the terminals drawn and the one with the
/// keys.
fn shown(panes: &Panes, cx: &mut VisualTestContext) -> Vec<yaar_link::TerminalId> {
    panes.pane.read_with(cx, |pane, _| pane.shown())
}

fn focused(panes: &Panes, cx: &mut VisualTestContext) -> Option<yaar_link::TerminalId> {
    panes.pane.read_with(cx, |pane, _| pane.focused())
}

#[gpui_kit::test]
fn the_pane_holds_several_terminals_at_once_and_switches_between_them(cx: &mut TestAppContext) {
    let (mut panes, mut cx) = open_panes(cx);
    let (t1, t2, t3) = (terminal('1'), terminal('2'), terminal('3'));

    // Three presses of the key of New Terminal, each after leaving the terminal.
    for _ in 0..3 {
        panes.press_in_the_app::<NewTerminal>(&mut cx);
    }
    // The questions come in the order they were asked and may be answered in any order:
    // the terminals are made as the answers come, each its own, and none is lost.
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
    // The second terminal's output arrives while the third is shown.
    panes.keeper.output(&t3, b"three$");
    panes.keeper.output(&t2, b"two$");
    wait_until(&mut cx, "the output of every terminal", |cx| {
        [(&t1, "one$"), (&t2, "two$"), (&t3, "three$")]
            .into_iter()
            .all(|(id, text)| lines(&panes.pane, id, cx).first().map(String::as_str) == Some(text))
    });

    // Holds several terminals at once.
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
    assert_eq!(lines(&panes.pane, &t1, &mut cx)[0], "one$");
    assert_eq!(lines(&panes.pane, &t2, &mut cx)[0], "two$");
    assert_eq!(lines(&panes.pane, &t3, &mut cx)[0], "three$");

    // Switches between them: the last one opened is shown and has the focus.
    assert_eq!(shown(&panes, &mut cx), [t3.clone()]);
    assert_eq!(focused(&panes, &mut cx), Some(t3.clone()));
    assert!(has_the_focus(&panes, &t3, &mut cx));

    let step =
        |panes: &mut Panes, cx: &mut VisualTestContext, key: &str, now: &yaar_link::TerminalId| {
            panes.click_elsewhere(cx);
            cx.simulate_keystrokes(key);
            assert_eq!(shown(panes, cx), [now.clone()], "after {key}");
            assert_eq!(focused(panes, cx), Some(now.clone()), "after {key}");
            assert!(has_the_focus(panes, now, cx), "after {key}");
        };
    // Round from the last to the first, and back.
    step(&mut panes, &mut cx, key_of::<NextTerminal>(), &t1);
    assert_eq!(lines(&panes.pane, &t1, &mut cx)[0], "one$");
    step(&mut panes, &mut cx, key_of::<PreviousTerminal>(), &t3);
    assert_eq!(lines(&panes.pane, &t3, &mut cx)[0], "three$");
    step(&mut panes, &mut cx, key_of::<PreviousTerminal>(), &t2);
    assert_eq!(lines(&panes.pane, &t2, &mut cx)[0], "two$");
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
    assert_eq!(two.len(), 2);
    assert_ne!(two[0], two[1]);
    let (first, second) = (
        drawn("terminal-slot-0", &mut cx).expect("the first slot is drawn"),
        drawn("terminal-slot-1", &mut cx).expect("the second slot is drawn"),
    );
    assert!(
        first.right() <= second.left(),
        "the slots are not side by side: {first:?} and {second:?}"
    );

    // Keys go to the terminal that has the focus: the one of the slot clicked.
    panes.click_slot(1, &mut cx);
    assert_eq!(focused(&panes, &mut cx), Some(two[1].clone()));
    cx.simulate_input("q");
    assert_eq!(panes.keeper.input(&mut cx), (two[1].clone(), b"q".to_vec()));

    panes.click_slot(0, &mut cx);
    assert_eq!(focused(&panes, &mut cx), Some(two[0].clone()));
    cx.simulate_input("w");
    assert_eq!(panes.keeper.input(&mut cx), (two[0].clone(), b"w".to_vec()));

    // No other frame of input came between.
    cx.run_until_parked();
    assert!(panes.keeper.no_input_waits());
}

/// How many frames the window was asked for since the last look, after the app has been
/// run until it has nothing left to do.
fn frames_asked(cx: &mut VisualTestContext) -> usize {
    let asked = cx.update(|window, cx| window.simulate_next_frame(cx));
    cx.run_until_parked();
    asked
}

/// Moves the clock of the executor on by 100 ms `steps` times and says that the window
/// asked for a frame after each.
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

#[gpui_kit::test]
fn after_each_key_the_pane_keeps_asking_for_frames_for_one_second(cx: &mut TestAppContext) {
    assert_eq!(FRAMES_AFTER_A_KEY, Duration::from_secs(1));
    let (mut panes, mut cx) = open_panes(cx);
    let one = terminal('1');
    panes.open_terminal(&one, &mut cx);
    assert!(has_the_focus(&panes, &one, &mut cx));
    // Settled: drawing again asks for nothing.
    while frames_asked(&mut cx) > 0 {}
    assert_eq!(frames_asked(&mut cx), 0);

    // A key typed in the terminal: a frame at every step up to 900 ms.
    cx.simulate_keystrokes("a");
    frames_keep_coming(9, 0, &mut cx);
    // Past the second the next frame is drawn, and it asks for none.
    cx.executor().advance_clock(Duration::from_millis(200));
    frames_asked(&mut cx);
    assert_eq!(frames_asked(&mut cx), 0, "frames go on after one second");

    // A key that runs an action never reaches a key listener, and counts all the same:
    // 500 ms after the first key it moves the end to 1,500 ms.
    cx.simulate_keystrokes("a");
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.simulate_keystrokes(key_of::<ScrollPageUp>());
    frames_keep_coming(9, 5, &mut cx);
    cx.executor().advance_clock(Duration::from_millis(200));
    frames_asked(&mut cx);
    assert_eq!(frames_asked(&mut cx), 0, "frames go on after 1,500 ms");
}

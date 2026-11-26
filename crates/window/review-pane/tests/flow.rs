//! Component test: Flow and its timeline, with the Keeper played by the test.
//! A negative clause is read after a barrier: a later message through the same path, whose arrival
//! shows that everything before it arrived.

mod common;

use std::time::{Duration, Instant};

use gpui_kit::{Modifiers, ScrollDelta, ScrollEvent, TestAppContext, VisualTestContext, point, px};
use yaar_link::{
    Fact, FileStatus, FlowForm, History, HistoryEntry, Reply, Request, Session, SessionState,
    Setting, Step, Turn, TurnEnd, TurnId,
};
use yaar_review_pane::{Following, TimelineEntry, View};

use common::{
    COMMIT, DEADLINE, Stage, agents_of, bounds_of_row, drawn, file, id, open, open_staged,
    running_turn, session, session_id, since_main, step_fact, texts, turn_id, wait_for_files,
    wait_until, working_on, workspace,
};

fn s0() -> String {
    "5".repeat(40)
}

fn s1() -> String {
    "6".repeat(40)
}

fn read(path: &str) -> Step {
    Step::Read {
        path: path.into(),
        lines: None,
    }
}

fn edit(path: &str, first: u32, last: u32) -> Step {
    Step::Edit {
        path: path.into(),
        // One range of lines: the edit tool names one.
        lines: Some(std::iter::once(first..=last).collect()),
    }
}

fn write(path: &str) -> Step {
    Step::Write { path: path.into() }
}

fn command(command: &str) -> Step {
    Step::Command {
        command: command.to_owned(),
    }
}

fn session_a_on(turn: &TurnId) -> Session {
    working_on(
        session('a', "Claude Code 1", SessionState::Works, None),
        turn.clone(),
    )
}

fn click(selector: &str, cx: &mut VisualTestContext) {
    let at = drawn(selector, cx)
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
        .center();
    cx.simulate_click(at, Modifiers::none());
    cx.run_until_parked();
}

fn press(keys: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
}

fn focus_by_the_bar(stage: &Stage, cx: &mut VisualTestContext) {
    click("review-bar", cx);
    cx.update(|window, _| assert!(stage.focus.is_focused(window)));
}

fn timeline(stage: &Stage, cx: &mut VisualTestContext) -> Vec<TimelineEntry> {
    stage.pane.read_with(cx, |pane, _| pane.timeline().to_vec())
}

fn view(stage: &Stage, cx: &mut VisualTestContext) -> View {
    stage.pane.read_with(cx, |pane, _| pane.view())
}

fn form(stage: &Stage, cx: &mut VisualTestContext) -> FlowForm {
    stage.pane.read_with(cx, |pane, _| pane.flow_form())
}

fn wait_for_timeline(stage: &Stage, wanted: &[TimelineEntry], cx: &mut VisualTestContext) {
    let deadline = Instant::now() + DEADLINE;
    loop {
        cx.run_until_parked();
        let held = timeline(stage, cx);
        if held == wanted {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "waited in vain for the timeline {wanted:#?}, which is {held:#?}"
        );
        std::thread::park_timeout(Duration::from_millis(1));
    }
}

fn heading(turn: &TurnId, session: char, title: &str) -> TimelineEntry {
    TimelineEntry::Turn {
        turn: turn.clone(),
        session: session_id(session),
        title: title.to_owned(),
    }
}

fn step_entry(turn: &TurnId, number: u32, step: Step) -> TimelineEntry {
    TimelineEntry::Step {
        turn: turn.clone(),
        number,
        step,
    }
}

fn end(turn: &TurnId, how: Option<TurnEnd>) -> TimelineEntry {
    TimelineEntry::End {
        turn: turn.clone(),
        how,
    }
}

fn asked_for(stage: &Stage, turn: &TurnId) -> usize {
    stage
        .keeper
        .turns_asked()
        .iter()
        .filter(|asked| *asked == turn)
        .count()
}

/// Answers what the Window asks first, so that the later requests are Flow's and the turn end's.
fn show_changes(stage: &mut Stage, cx: &mut VisualTestContext) {
    let changes = since_main(Vec::new(), vec![file("a.rs", FileStatus::Modified)]);
    stage.keeper.show(&changes, |_| texts("a\n", "b\n"), cx);
    wait_for_files(&stage.pane, cx);
}

fn wheel(stage: &Stage, by: f32, cx: &mut VisualTestContext) {
    let in_view = stage.pane.read_with(cx, |pane, _| pane.rows_in_view());
    let over_the_rows = bounds_of_row(in_view.start + 2, cx).center();
    cx.simulate_event(ScrollEvent {
        position: over_the_rows,
        delta: ScrollDelta::Pixels(point(px(0.), px(by))),
        ..ScrollEvent::default()
    });
    cx.run_until_parked();
}

fn reading(
    stage: &Stage,
    cx: &mut VisualTestContext,
) -> (std::ops::Range<usize>, Option<yaar_review_pane::Position>) {
    cx.run_until_parked();
    stage.pane.read_with(cx, |pane, _| {
        (pane.rows_in_view(), pane.position().cloned())
    })
}

#[gpui_kit::test]
fn the_pane_offers_flow_and_changes_and_switches_between_them(cx: &mut TestAppContext) {
    let (mut stage, mut cx) = open(vec![workspace('1', "feature")], cx);
    let lines: String = (0..40).map(|line| format!("line {line}\n")).collect();
    let changes = since_main(Vec::new(), vec![file("long.txt", FileStatus::Added)]);
    stage.keeper.show(&changes, |_| texts("", &lines), &mut cx);
    wait_for_files(&stage.pane, &mut cx);

    assert_eq!(view(&stage, &mut cx), View::Changes);
    for selector in ["review-bar", "review-view-changes", "review-view-flow"] {
        assert!(drawn(selector, &mut cx).is_some(), "{selector} is drawn");
    }

    click("review-row-2", &mut cx);
    cx.update(|window, _| assert!(stage.focus.is_focused(window)));
    for _ in 0..3 {
        press("j", &mut cx);
    }
    wheel(&stage, -10., &mut cx);
    let before = reading(&stage, &mut cx);
    let first_line = stage
        .pane
        .read_with(&cx, |pane, _| pane.position().cloned());
    assert_ne!(first_line.as_ref().map(|position| position.line), Some(0));

    press("f", &mut cx);
    assert_eq!(view(&stage, &mut cx), View::Flow);
    assert!(drawn("flow-empty", &mut cx).is_some());
    assert!(drawn("review-row-0", &mut cx).is_none());
    press("f", &mut cx);
    assert_eq!(view(&stage, &mut cx), View::Changes);
    assert!(drawn("review-row-0", &mut cx).is_some());
    assert!(drawn("flow-empty", &mut cx).is_none());
    assert_eq!(reading(&stage, &mut cx), before);

    for _ in 0..30 {
        press("j", &mut cx);
    }
    wheel(&stage, -40., &mut cx);
    let (in_view, position) = reading(&stage, &mut cx);
    assert!(in_view.start > 0, "{in_view:?} is below the top");
    assert!(drawn("review-row-0", &mut cx).is_none());
    click("review-view-flow", &mut cx);
    assert_eq!(view(&stage, &mut cx), View::Flow);
    assert!(drawn("flow-empty", &mut cx).is_some());
    click("review-view-changes", &mut cx);
    assert_eq!(view(&stage, &mut cx), View::Changes);
    assert_eq!(reading(&stage, &mut cx), (in_view.clone(), position));
    assert!(drawn(&format!("review-row-{}", in_view.start), &mut cx).is_some());
    click("review-view-changes", &mut cx);
    assert_eq!(view(&stage, &mut cx), View::Changes);
}

#[gpui_kit::test]
fn the_timeline_lists_each_edit_read_and_command_of_the_turn_in_order(cx: &mut TestAppContext) {
    let t = turn_id('t');
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature")],
        |keeper| {
            keeper.set_settings(vec![Setting::FlowForm {
                workspace: id('1'),
                form: FlowForm::Timeline,
            }]);
            keeper.set_agents(vec![agents_of(vec![session_a_on(&t)])]);
            keeper.set_turn(running_turn(
                turn_id('t'),
                session_id('a'),
                1,
                Some(&s0()),
                vec![read("notes.txt"), edit("src/a.rs", 3, 4)],
            ));
        },
        cx,
    );
    show_changes(&mut stage, &mut cx);
    focus_by_the_bar(&stage, &mut cx);
    press("f", &mut cx);

    let so_far = [
        heading(&t, 'a', "Claude Code 1"),
        step_entry(&t, 1, read("notes.txt")),
        step_entry(&t, 2, edit("src/a.rs", 3, 4)),
    ];
    wait_for_timeline(&stage, &so_far, &mut cx);
    assert_eq!(stage.keeper.turns_asked(), std::slice::from_ref(&t));
    for index in 0..so_far.len() {
        assert!(drawn(&format!("flow-entry-{index}"), &mut cx).is_some());
    }
    assert!(drawn("flow-entry-3", &mut cx).is_none());
    assert!(drawn("flow-empty", &mut cx).is_none());
    for (index, words) in ["Claude Code 1", "Read notes.txt", "Edit src/a.rs lines 3-4"]
        .iter()
        .enumerate()
    {
        assert!(
            drawn(&format!("flow-entry-text-{index}:{words}"), &mut cx).is_some(),
            "entry {index} says {words}"
        );
    }

    let s = session_id('a');
    stage
        .keeper
        .tell(step_fact(s.clone(), t.clone(), 3, write("new.txt")));
    stage
        .keeper
        .tell(step_fact(s.clone(), t.clone(), 3, write("new.txt")));
    stage.keeper.tell(step_fact(
        s.clone(),
        t.clone(),
        4,
        command("echo x > shell.txt"),
    ));
    let four = [
        so_far[0].clone(),
        so_far[1].clone(),
        so_far[2].clone(),
        step_entry(&t, 3, write("new.txt")),
        step_entry(&t, 4, command("echo x > shell.txt")),
    ];
    wait_for_timeline(&stage, &four, &mut cx);
    assert_eq!(asked_for(&stage, &t), 1);

    // A step with one missing before it has the turn read again, and the answer fills the gap.
    let steps = vec![
        read("notes.txt"),
        edit("src/a.rs", 3, 4),
        write("new.txt"),
        command("echo x > shell.txt"),
        read("five.txt"),
        write("six.txt"),
    ];
    stage.keeper.set_turn(running_turn(
        t.clone(),
        s.clone(),
        1,
        Some(&s0()),
        steps.clone(),
    ));
    stage
        .keeper
        .tell(step_fact(s.clone(), t.clone(), 6, write("six.txt")));
    let mut all = vec![heading(&t, 'a', "Claude Code 1")];
    all.extend(
        steps
            .iter()
            .enumerate()
            .map(|(index, step)| step_entry(&t, index as u32 + 1, step.clone())),
    );
    wait_for_timeline(&stage, &all, &mut cx);
    assert_eq!(asked_for(&stage, &t), 2);

    // A step of another workspace is no step of this one: told first, it would take the place of
    // the step of this workspace with the same number.
    let mut other = step_fact(s.clone(), t.clone(), 7, read("elsewhere.txt"));
    if let Fact::Step { workspace, .. } = &mut other {
        *workspace = id('2');
    }
    stage.keeper.tell(other);
    stage
        .keeper
        .tell(step_fact(s.clone(), t.clone(), 7, command("later")));
    all.push(step_entry(&t, 7, command("later")));
    wait_for_timeline(&stage, &all, &mut cx);

    stage.keeper.tell(Fact::TurnEnded {
        workspace: id('1'),
        session: s,
        turn: t.clone(),
        state: Some(s1()),
        since: None,
    });
    all.push(end(&t, None));
    wait_for_timeline(&stage, &all, &mut cx);
    let (asked, request) = stage.keeper.request(&mut cx);
    assert!(
        matches!(request, Request::CompareVersions { .. }),
        "{request:?}"
    );
    stage.keeper.reply(asked, Ok(Reply::Compared(Vec::new())));
    assert_eq!(view(&stage, &mut cx), View::Flow);
}

#[gpui_kit::test]
fn a_pane_opened_where_no_turn_runs_lists_the_newest_turn_of_the_history_and_the_turns_that_start_after_it(
    cx: &mut TestAppContext,
) {
    let t0 = turn_id('0');
    let t = turn_id('t');
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature")],
        |keeper| {
            keeper.set_agents(vec![agents_of(vec![session(
                'a',
                "Claude Code 1",
                SessionState::Idle,
                None,
            )])]);
            keeper.set_history(History {
                workspace: id('1'),
                start: Some(COMMIT.to_owned()),
                entries: vec![
                    // Turns with no state kept: Changes has none to select, which would be the
                    // first request.
                    HistoryEntry::Turn {
                        turn: turn_id('9'),
                        session: session_id('a'),
                        state: None,
                    },
                    HistoryEntry::Turn {
                        turn: turn_id('0'),
                        session: session_id('a'),
                        state: None,
                    },
                ],
            });
            keeper.set_turn(Turn {
                ended: Some(TurnEnd::Finished),
                state: None,
                ..running_turn(
                    turn_id('0'),
                    session_id('a'),
                    2,
                    Some(&s0()),
                    vec![read("old.txt"), write("old-new.txt")],
                )
            });
        },
        cx,
    );
    show_changes(&mut stage, &mut cx);
    focus_by_the_bar(&stage, &mut cx);
    press("f", &mut cx);

    let first = vec![
        heading(&t0, 'a', "Claude Code 1"),
        step_entry(&t0, 1, read("old.txt")),
        step_entry(&t0, 2, write("old-new.txt")),
        end(&t0, Some(TurnEnd::Finished)),
    ];
    wait_for_timeline(&stage, &first, &mut cx);
    assert_eq!(stage.keeper.turns_asked(), std::slice::from_ref(&t0));

    stage.keeper.set_turn(running_turn(
        t.clone(),
        session_id('a'),
        3,
        Some(&s1()),
        Vec::new(),
    ));
    stage.keeper.tell_agents(agents_of(vec![session_a_on(&t)]));
    let mut both = first;
    both.push(heading(&t, 'a', "Claude Code 1"));
    wait_for_timeline(&stage, &both, &mut cx);
    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 1, command("make")));
    both.push(step_entry(&t, 1, command("make")));
    wait_for_timeline(&stage, &both, &mut cx);
}

#[gpui_kit::test]
fn a_turn_whose_recording_expired_is_not_the_turn_flow_lists_when_it_opens(
    cx: &mut TestAppContext,
) {
    let t0 = turn_id('0');
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature")],
        |keeper| {
            keeper.set_agents(vec![agents_of(vec![session(
                'a',
                "Claude Code 1",
                SessionState::Idle,
                None,
            )])]);
            keeper.set_history(History {
                workspace: id('1'),
                start: Some(COMMIT.to_owned()),
                entries: vec![
                    HistoryEntry::Turn {
                        turn: turn_id('0'),
                        session: session_id('a'),
                        state: None,
                    },
                    HistoryEntry::ExpiredTurn {
                        turn: turn_id('9'),
                        session: session_id('a'),
                    },
                ],
            });
            keeper.set_turn(Turn {
                ended: Some(TurnEnd::Finished),
                state: None,
                ..running_turn(
                    turn_id('0'),
                    session_id('a'),
                    2,
                    Some(&s0()),
                    vec![read("old.txt")],
                )
            });
        },
        cx,
    );
    show_changes(&mut stage, &mut cx);
    focus_by_the_bar(&stage, &mut cx);
    press("f", &mut cx);
    wait_for_timeline(
        &stage,
        &[
            heading(&t0, 'a', "Claude Code 1"),
            step_entry(&t0, 1, read("old.txt")),
            end(&t0, Some(TurnEnd::Finished)),
        ],
        &mut cx,
    );
    assert_eq!(stage.keeper.turns_asked(), std::slice::from_ref(&t0));
}

#[gpui_kit::test]
fn a_turn_the_keeper_does_not_hold_keeps_its_heading_and_says_nothing(cx: &mut TestAppContext) {
    let t = turn_id('t');
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature")],
        |keeper| keeper.set_agents(vec![agents_of(vec![session_a_on(&turn_id('t'))])]),
        cx,
    );
    show_changes(&mut stage, &mut cx);
    focus_by_the_bar(&stage, &mut cx);
    press("f", &mut cx);
    wait_for_timeline(&stage, &[heading(&t, 'a', "Claude Code 1")], &mut cx);
    wait_until(&mut cx, "the question for the turn", |_| {
        asked_for(&stage, &t) == 1
    });

    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 1, read("x.txt")));
    wait_for_timeline(
        &stage,
        &[
            heading(&t, 'a', "Claude Code 1"),
            step_entry(&t, 1, read("x.txt")),
        ],
        &mut cx,
    );
    assert!(drawn("review-notice", &mut cx).is_none());
    assert!(drawn("review-refused", &mut cx).is_none());
}

#[gpui_kit::test]
fn j_and_k_move_the_reviewer_through_the_entries_and_stay_at_the_first_and_the_last(
    cx: &mut TestAppContext,
) {
    let t = turn_id('t');
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature")],
        |keeper| {
            keeper.set_settings(vec![Setting::FlowForm {
                workspace: id('1'),
                form: FlowForm::Timeline,
            }]);
            keeper.set_agents(vec![agents_of(vec![session_a_on(&turn_id('t'))])]);
            keeper.set_turn(running_turn(
                turn_id('t'),
                session_id('a'),
                1,
                Some(&s0()),
                vec![read("a.txt"), read("b.txt")],
            ));
        },
        cx,
    );
    show_changes(&mut stage, &mut cx);
    focus_by_the_bar(&stage, &mut cx);
    press("f", &mut cx);
    wait_until(&mut cx, "the three entries", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.timeline().len() == 3)
    });
    let cursor = |stage: &Stage, cx: &mut VisualTestContext| {
        stage.pane.read_with(cx, |pane, _| pane.timeline_cursor())
    };
    assert_eq!(cursor(&stage, &mut cx), None);
    press("j", &mut cx);
    assert_eq!(cursor(&stage, &mut cx), Some(0));
    press("j", &mut cx);
    press("j", &mut cx);
    press("j", &mut cx);
    assert_eq!(cursor(&stage, &mut cx), Some(2), "it stays at the last");
    press("k", &mut cx);
    assert_eq!(cursor(&stage, &mut cx), Some(1));
    press("k", &mut cx);
    press("k", &mut cx);
    assert_eq!(cursor(&stage, &mut cx), Some(0), "it stays at the first");

    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 3, read("c.txt")));
    wait_until(&mut cx, "the fourth entry", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.timeline().len() == 4)
    });
    assert_eq!(cursor(&stage, &mut cx), Some(0));
    click("flow-entry-2", &mut cx);
    assert_eq!(cursor(&stage, &mut cx), Some(2));
}

#[gpui_kit::test]
fn the_list_of_a_long_timeline_scrolls_with_the_reviewer_and_keeps_its_place_across_the_views(
    cx: &mut TestAppContext,
) {
    let steps: Vec<Step> = (0..40)
        .map(|number| read(&format!("file-{number}.txt")))
        .collect();
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature")],
        |keeper| {
            keeper.set_settings(vec![Setting::FlowForm {
                workspace: id('1'),
                form: FlowForm::Timeline,
            }]);
            keeper.set_agents(vec![agents_of(vec![session_a_on(&turn_id('t'))])]);
            keeper.set_turn(running_turn(
                turn_id('t'),
                session_id('a'),
                1,
                Some(&s0()),
                steps.clone(),
            ));
        },
        cx,
    );
    show_changes(&mut stage, &mut cx);
    focus_by_the_bar(&stage, &mut cx);
    press("f", &mut cx);
    wait_until(&mut cx, "the entries", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.timeline().len() == 41)
    });
    let in_view = |stage: &Stage, cx: &mut VisualTestContext| {
        cx.run_until_parked();
        stage.pane.read_with(cx, |pane, _| pane.flow_rows_in_view())
    };
    let at_first = in_view(&stage, &mut cx);
    assert_eq!(at_first.start, 0);
    assert!(at_first.end < 41, "{at_first:?} leaves entries out of view");
    assert!(drawn("flow-entry-0", &mut cx).is_some());
    assert!(drawn("flow-entry-40", &mut cx).is_none());

    for _ in 0..35 {
        press("j", &mut cx);
    }
    let cursor = stage
        .pane
        .read_with(&cx, |pane, _| pane.timeline_cursor())
        .expect("the reviewer is on an entry");
    assert_eq!(cursor, 34);
    assert!(in_view(&stage, &mut cx).contains(&cursor));
    assert!(drawn(&format!("flow-entry-{cursor}"), &mut cx).is_some());
    assert!(drawn("flow-entry-0", &mut cx).is_none());

    let there = in_view(&stage, &mut cx);
    press("f", &mut cx);
    press("f", &mut cx);
    assert_eq!(in_view(&stage, &mut cx), there);
    for _ in 0..40 {
        press("k", &mut cx);
    }
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.following()),
        Following::Paused { missed: 0 }
    );
    assert_eq!(in_view(&stage, &mut cx), at_first);
    assert!(drawn("flow-entry-0", &mut cx).is_some());
}

#[gpui_kit::test]
fn a_turn_that_started_before_its_start_was_kept_is_read_again_at_a_turn_end_and_one_with_a_start_is_not(
    cx: &mut TestAppContext,
) {
    let (t1, t2, t3) = (turn_id('1'), turn_id('2'), turn_id('3'));
    let sessions = vec![
        working_on(
            session('a', "Claude Code 1", SessionState::Works, None),
            t1.clone(),
        ),
        working_on(
            session('b', "Claude Code 2", SessionState::Works, None),
            t2.clone(),
        ),
        working_on(
            session('c', "Claude Code 3", SessionState::Works, None),
            t3.clone(),
        ),
    ];
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature")],
        |keeper| {
            keeper.set_agents(vec![agents_of(sessions.clone())]);
            keeper.set_turn(running_turn(
                turn_id('1'),
                session_id('a'),
                1,
                Some(&s0()),
                vec![],
            ));
            // Turn 2 started while the state before it was still being recorded: no start.
            keeper.set_turn(running_turn(turn_id('2'), session_id('b'), 1, None, vec![]));
            keeper.set_turn(running_turn(
                turn_id('3'),
                session_id('c'),
                1,
                Some(&s0()),
                vec![],
            ));
        },
        cx,
    );
    show_changes(&mut stage, &mut cx);
    wait_until(&mut cx, "each turn read once", |_| {
        stage.keeper.turns_asked().len() == 3
    });

    stage.keeper.set_turn(running_turn(
        t2.clone(),
        session_id('b'),
        1,
        Some(&s1()),
        vec![],
    ));
    stage.keeper.tell(Fact::TurnEnded {
        workspace: id('1'),
        session: session_id('a'),
        turn: t1.clone(),
        state: None,
        since: None,
    });
    // The barrier is the form `shift-l` sets: the Window sends it after everything it sent for the
    // turn end, on the one connection.
    focus_by_the_bar(&stage, &mut cx);
    press("f", &mut cx);
    press("shift-l", &mut cx);
    wait_until(&mut cx, "the form to reach the Keeper", |_| {
        !stage.keeper.settings_set().is_empty()
    });
    wait_until(&mut cx, "turn 2 read again", |_| {
        asked_for(&stage, &t2) == 2
    });
    assert_eq!(asked_for(&stage, &t1), 1);
    assert_eq!(asked_for(&stage, &t3), 1);
    assert_eq!(asked_for(&stage, &t2), 2);
}

#[gpui_kit::test]
fn f_shift_l_and_a_click_on_the_bar_leave_follow_on_while_a_turn_runs(cx: &mut TestAppContext) {
    let t = turn_id('t');
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature")],
        |keeper| {
            keeper.set_agents(vec![agents_of(vec![session_a_on(&turn_id('t'))])]);
            keeper.set_turn(running_turn(
                turn_id('t'),
                session_id('a'),
                1,
                Some(&s0()),
                vec![],
            ));
        },
        cx,
    );
    show_changes(&mut stage, &mut cx);
    wait_until(&mut cx, "the turn read", |_| asked_for(&stage, &t) == 1);
    let following = |stage: &Stage, cx: &mut VisualTestContext| {
        stage.pane.read_with(cx, |pane, _| pane.following())
    };
    focus_by_the_bar(&stage, &mut cx);
    assert_eq!(following(&stage, &mut cx), Following::On);
    press("f", &mut cx);
    assert_eq!(following(&stage, &mut cx), Following::On);
    press("shift-l", &mut cx);
    assert_eq!(following(&stage, &mut cx), Following::On);
    click("review-bar", &mut cx);
    assert_eq!(following(&stage, &mut cx), Following::On);
    click("flow-form-timeline", &mut cx);
    click("review-view-changes", &mut cx);
    click("review-view-flow", &mut cx);
    assert_eq!(following(&stage, &mut cx), Following::On);
    press("f", &mut cx);
    assert_eq!(following(&stage, &mut cx), Following::On);
}

#[gpui_kit::test]
fn shift_l_with_changes_on_screen_switches_no_form_and_sends_nothing(cx: &mut TestAppContext) {
    let (mut stage, mut cx) = open(vec![workspace('1', "feature")], cx);
    show_changes(&mut stage, &mut cx);
    click("review-row-1", &mut cx);
    cx.update(|window, _| assert!(stage.focus.is_focused(window)));
    press("shift-l", &mut cx);
    assert_eq!(form(&stage, &mut cx), FlowForm::LiveDiff);
    // The barrier: Flow's own `shift-l` sends its setting after anything the first sent.
    press("f", &mut cx);
    press("shift-l", &mut cx);
    wait_until(&mut cx, "the form to reach the Keeper", |_| {
        !stage.keeper.settings_set().is_empty()
    });
    assert_eq!(
        stage.keeper.settings_set(),
        [Setting::FlowForm {
            workspace: id('1'),
            form: FlowForm::Timeline
        }]
    );
}

#[gpui_kit::test]
fn flow_opens_as_the_live_diff_the_first_time_and_the_two_forms_switch_at_any_time(
    cx: &mut TestAppContext,
) {
    let t = turn_id('t');
    let (mut stage, mut cx) = open(vec![workspace('1', "feature"), workspace('2', "other")], cx);
    let first = since_main(Vec::new(), vec![file("a.rs", FileStatus::Modified)]);
    stage.keeper.show(&first, |_| texts("a\n", "b\n"), &mut cx);
    wait_for_files(&stage.pane, &mut cx);

    focus_by_the_bar(&stage, &mut cx);
    press("f", &mut cx);
    assert_eq!(form(&stage, &mut cx), FlowForm::LiveDiff);
    assert!(drawn("flow-empty", &mut cx).is_some());
    assert!(drawn("flow-live", &mut cx).is_some());

    press("shift-l", &mut cx);
    assert_eq!(form(&stage, &mut cx), FlowForm::Timeline);
    let timeline_setting = Setting::FlowForm {
        workspace: id('1'),
        form: FlowForm::Timeline,
    };
    wait_until(&mut cx, "the form to reach the Keeper", |_| {
        stage.keeper.settings_set() == [timeline_setting.clone()]
    });
    assert!(drawn("flow-live", &mut cx).is_none());

    press("f", &mut cx);
    press("f", &mut cx);
    assert_eq!(form(&stage, &mut cx), FlowForm::Timeline);
    stage
        .models
        .workspaces
        .update(&mut cx, |list, cx| list.select(&id('2'), cx));
    let second = yaar_link::Changes {
        workspace: id('2'),
        ..first.clone()
    };
    stage.keeper.show(&second, |_| texts("a\n", "b\n"), &mut cx);
    wait_until(&mut cx, "the form of the other workspace", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.flow_form() == FlowForm::LiveDiff)
    });
    assert!(drawn("flow-live", &mut cx).is_some());
    stage
        .models
        .workspaces
        .update(&mut cx, |list, cx| list.select(&id('1'), cx));
    stage.keeper.show(&first, |_| texts("a\n", "b\n"), &mut cx);
    wait_until(&mut cx, "the form of the first workspace again", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.flow_form() == FlowForm::Timeline)
    });
    assert_eq!(view(&stage, &mut cx), View::Flow);

    stage.keeper.set_turn(running_turn(
        t.clone(),
        session_id('a'),
        1,
        Some(&s0()),
        vec![read("notes.txt"), edit("src/a.rs", 3, 4)],
    ));
    stage.keeper.tell_agents(agents_of(vec![session_a_on(&t)]));
    let two = [
        heading(&t, 'a', "Claude Code 1"),
        step_entry(&t, 1, read("notes.txt")),
        step_entry(&t, 2, edit("src/a.rs", 3, 4)),
    ];
    wait_for_timeline(&stage, &two, &mut cx);
    press("shift-l", &mut cx);
    assert_eq!(form(&stage, &mut cx), FlowForm::LiveDiff);
    assert!(drawn("flow-live", &mut cx).is_some());
    assert!(drawn("flow-entry-0", &mut cx).is_none());
    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 3, write("new.txt")));
    let mut three = two.to_vec();
    three.push(step_entry(&t, 3, write("new.txt")));
    wait_for_timeline(&stage, &three, &mut cx);
    assert_eq!(form(&stage, &mut cx), FlowForm::LiveDiff);
    assert!(drawn("flow-live", &mut cx).is_some());
    assert!(drawn("flow-entry-0", &mut cx).is_none());
    press("shift-l", &mut cx);
    assert_eq!(form(&stage, &mut cx), FlowForm::Timeline);
    assert_eq!(timeline(&stage, &mut cx), three);
    assert!(drawn("flow-entry-0", &mut cx).is_some());
    let timeline_form = Setting::FlowForm {
        workspace: id('1'),
        form: FlowForm::Timeline,
    };
    wait_until(&mut cx, "the last form to reach the Keeper", |_| {
        stage.keeper.settings_set().last() == Some(&timeline_form)
    });
    click("flow-form-live", &mut cx);
    assert_eq!(form(&stage, &mut cx), FlowForm::LiveDiff);
    click("flow-form-live", &mut cx);
    assert_eq!(form(&stage, &mut cx), FlowForm::LiveDiff);
    click("flow-form-timeline", &mut cx);
    assert_eq!(form(&stage, &mut cx), FlowForm::Timeline);
}

#[gpui_kit::test]
fn afterwards_flow_opens_in_the_form_chosen_last_in_a_new_window(cx: &mut TestAppContext) {
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature")],
        |keeper| {
            keeper.set_settings(vec![Setting::FlowForm {
                workspace: id('1'),
                form: FlowForm::Timeline,
            }]);
        },
        cx,
    );
    show_changes(&mut stage, &mut cx);
    focus_by_the_bar(&stage, &mut cx);
    press("f", &mut cx);
    wait_until(&mut cx, "the form the Keeper holds", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.flow_form() == FlowForm::Timeline)
    });
    assert!(drawn("flow-live", &mut cx).is_none());

    stage.keeper.tell(Fact::SettingChanged(Setting::FlowForm {
        workspace: id('1'),
        form: FlowForm::LiveDiff,
    }));
    wait_until(&mut cx, "the form another window chose", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.flow_form() == FlowForm::LiveDiff)
    });
    assert!(drawn("flow-live", &mut cx).is_some());
    // The form another window chose is no choice of this window's, so nothing was sent. The barrier
    // is this window's own choice, which reaches the Keeper after anything the told form sent.
    press("shift-l", &mut cx);
    let own = Setting::FlowForm {
        workspace: id('1'),
        form: FlowForm::Timeline,
    };
    wait_until(&mut cx, "the form chosen here to reach the Keeper", |_| {
        !stage.keeper.settings_set().is_empty()
    });
    assert_eq!(stage.keeper.settings_set(), [own]);
}

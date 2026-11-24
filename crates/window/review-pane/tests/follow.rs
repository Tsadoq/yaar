//! Component test: follow and the offer at a turn end, with the Keeper played by the test.
//! A negative clause is read after a barrier: a later message through the same path.
//! Facts reach the pane by a thread of their own: every wait is for a fact.

mod common;

use std::ops::Range;
use std::path::PathBuf;

use gpui_kit::{
    AppContext, Modifiers, ScrollDelta, ScrollEvent, TestAppContext, VisualTestContext, point, px,
};
use yaar_link::{
    ChangedFile, CreditedFile, Fact, FileStatus, FlowForm, History, Reply, Request, Session,
    SessionState, Step, TurnId,
};
use yaar_review_pane::{ChangeShown, FlowAt, Following, PaneRow, TimelineEntry, View};

use common::{
    Stage, agents_of, drawn, file, id, open, open_staged, review_id, review_with, reviews_of,
    running_turn, self_started, session, session_id, since_main, step_fact, texts, turn_id,
    wait_for_files, wait_until, working_on, workspace,
};

fn s0() -> String {
    "5".repeat(40)
}

fn s1() -> String {
    "6".repeat(40)
}

fn s2() -> String {
    "7".repeat(40)
}

/// Where the history starts: not the commit the changes are counted from, so a count from the
/// wrong one shows.
fn history_start() -> String {
    "8".repeat(40)
}

fn read(path: &str) -> Step {
    Step::Read { path: path.into() }
}

fn write(path: &str) -> Step {
    Step::Write { path: path.into() }
}

fn command(command: &str) -> Step {
    Step::Command {
        command: command.to_owned(),
    }
}

fn alternating(number: u32) -> Step {
    match number % 2 {
        1 => read(&format!("r{number}.txt")),
        _ => write(&format!("w{number}.txt")),
    }
}

fn session_a_on(turn: &TurnId) -> Session {
    working_on(
        session('a', "Claude Code 1", SessionState::Works, Some(review_id())),
        turn.clone(),
    )
}

fn session_a_idle() -> Session {
    session('a', "Claude Code 1", SessionState::Idle, Some(review_id()))
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

/// A click on the bar focuses the pane without pausing follow.
fn focus_by_the_bar(stage: &Stage, cx: &mut VisualTestContext) {
    click("review-bar", cx);
    cx.update(|window, _| assert!(stage.focus.is_focused(window)));
}

fn following<C: AppContext>(stage: &Stage, cx: &C) -> Following {
    stage.pane.read_with(cx, |pane, _| pane.following())
}

fn view<C: AppContext>(stage: &Stage, cx: &C) -> View {
    stage.pane.read_with(cx, |pane, _| pane.view())
}

fn flow_at<C: AppContext>(stage: &Stage, cx: &C) -> Option<FlowAt> {
    stage.pane.read_with(cx, |pane, _| pane.flow_at().cloned())
}

fn flow_offer<C: AppContext>(stage: &Stage, cx: &C) -> Option<ChangeShown> {
    stage
        .pane
        .read_with(cx, |pane, _| pane.flow_offer().cloned())
}

fn notice<C: AppContext>(stage: &Stage, cx: &C) -> Option<String> {
    stage.pane.read_with(cx, |pane, _| pane.notice())
}

fn live<C: AppContext>(stage: &Stage, cx: &C) -> (Vec<PaneRow>, Option<usize>) {
    stage.pane.read_with(cx, |pane, _| {
        (pane.live_rows().to_vec(), pane.flow_at_row())
    })
}

fn at(turn: &TurnId, number: u32) -> FlowAt {
    FlowAt {
        turn: turn.clone(),
        number,
    }
}

fn wait_for_step(stage: &Stage, turn: &TurnId, number: u32, cx: &mut VisualTestContext) {
    wait_until(cx, "the step in the timeline", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.timeline().iter().any(|entry| {
                matches!(entry, TimelineEntry::Step { turn: held, number: held_number, .. }
                    if held == turn && *held_number == number)
            })
        })
    });
}

fn wait_for_at(stage: &Stage, turn: &TurnId, number: u32, cx: &mut VisualTestContext) {
    let wanted = at(turn, number);
    wait_until(cx, "Flow at the step", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.flow_at() == Some(&wanted))
    });
}

fn wait_for_its_row_in_view(stage: &Stage, cx: &mut VisualTestContext) -> usize {
    let mut found = None;
    wait_until(cx, "the row of the step in view", |cx| {
        let (row, in_view) = stage
            .pane
            .read_with(cx, |pane, _| (pane.flow_at_row(), pane.flow_rows_in_view()));
        found = row.filter(|row| in_view.contains(row));
        found.is_some()
    });
    found.expect("waited for it")
}

fn entry_of(stage: &Stage, turn: &TurnId, number: u32, cx: &mut VisualTestContext) -> usize {
    stage.pane.read_with(cx, |pane, _| {
        pane.timeline()
            .iter()
            .position(|entry| {
                matches!(entry, TimelineEntry::Step { turn: held, number: held_number, .. }
                    if held == turn && *held_number == number)
            })
            .expect("the timeline holds the step")
    })
}

fn in_view(stage: &Stage, cx: &mut VisualTestContext) -> Range<usize> {
    cx.run_until_parked();
    stage.pane.read_with(cx, |pane, _| pane.flow_rows_in_view())
}

fn show_changes(stage: &mut Stage, cx: &mut VisualTestContext) {
    let changes = since_main(Vec::new(), vec![file("a.rs", FileStatus::Modified)]);
    stage.keeper.show(&changes, |_| texts("a\n", "b\n"), cx);
    wait_for_files(&stage.pane, cx);
}

fn wheel(stage: &Stage, by: f32, cx: &mut VisualTestContext) {
    let in_view = in_view(stage, cx);
    let over_the_list = drawn(&format!("flow-entry-{}", in_view.start + 1), cx)
        .expect("an entry is in view")
        .center();
    cx.simulate_event(ScrollEvent {
        position: over_the_list,
        delta: ScrollDelta::Pixels(point(px(0.), px(by))),
        ..ScrollEvent::default()
    });
    cx.run_until_parked();
}

fn click_an_entry(stage: &Stage, cx: &mut VisualTestContext) {
    let in_view = in_view(stage, cx);
    click(&format!("flow-entry-{}", in_view.start + 1), cx);
}

fn at_work_in_flow(steps: Vec<Step>, cx: &mut TestAppContext) -> (Stage, VisualTestContext) {
    at_work_in_flow_with(steps, None, cx)
}

fn at_work_in_flow_with(
    steps: Vec<Step>,
    start: Option<String>,
    cx: &mut TestAppContext,
) -> (Stage, VisualTestContext) {
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature"), workspace('2', "other")],
        |keeper| {
            if let Some(start) = start {
                keeper.set_history(History {
                    workspace: id('1'),
                    start: Some(start),
                    entries: Vec::new(),
                });
            }
            keeper.set_agents(vec![agents_of(vec![session_a_on(&turn_id('t'))])]);
            keeper.set_turn(running_turn(
                turn_id('t'),
                session_id('a'),
                1,
                Some(&s0()),
                steps,
            ));
            keeper.set_reviews(reviews_of(vec![review_with(
                review_id(),
                Some(session_id('a')),
                Some(s1()),
            )]));
        },
        cx,
    );
    show_changes(&mut stage, &mut cx);
    focus_by_the_bar(&stage, &mut cx);
    press("f", &mut cx);
    assert_eq!(view(&stage, &cx), View::Flow);
    assert_eq!(following(&stage, &cx), Following::On);
    (stage, cx)
}

fn tell_step(stage: &mut Stage, t: &TurnId, number: u32, cx: &mut VisualTestContext) {
    stage.keeper.tell(step_fact(
        session_id('a'),
        t.clone(),
        number,
        alternating(number),
    ));
    wait_for_step(stage, t, number, cx);
}

fn tell_the_step(
    stage: &mut Stage,
    t: &TurnId,
    number: u32,
    step: Step,
    cx: &mut VisualTestContext,
) {
    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), number, step));
    wait_for_step(stage, t, number, cx);
}

fn compared(
    stage: &mut Stage,
    from: &str,
    to: Option<&str>,
    files: Vec<ChangedFile>,
    cx: &mut VisualTestContext,
) {
    let asked = stage.keeper.expect(
        Request::CompareVersions {
            workspace: id('1'),
            from: from.to_owned(),
            to: to.map(str::to_owned),
        },
        cx,
    );
    stage.keeper.reply(asked, Ok(Reply::Compared(files)));
}

fn read_versions_of(
    stage: &mut Stage,
    old: &str,
    new: Option<&str>,
    (name, before, after): (&str, &str, &str),
    cx: &mut VisualTestContext,
) {
    let (asked, request) = stage.keeper.request(cx);
    let Request::ReadVersions {
        workspace,
        old: from,
        old_path,
        new: to,
        path,
    } = request
    else {
        panic!("a read of the versions of a file was expected, not {request:?}");
    };
    assert_eq!(workspace, id('1'));
    assert_eq!(from.as_deref(), Some(old));
    assert_eq!(to.as_deref(), new);
    assert_eq!((old_path, path), (PathBuf::from(name), PathBuf::from(name)));
    stage
        .keeper
        .reply(asked, texts(before, after).map(Reply::File));
}

fn turn_ended(t: &TurnId, state: Option<String>, since: Option<String>) -> Fact {
    Fact::TurnEnded {
        workspace: id('1'),
        session: session_id('a'),
        turn: t.clone(),
        state,
        since,
    }
}

fn the_turn_end_selects(stage: &mut Stage, from: &str, to: &str, cx: &mut VisualTestContext) {
    let asked = stage.keeper.expect(
        Request::CompareForReview {
            workspace: id('1'),
            review: review_id(),
            from: from.to_owned(),
            to: to.to_owned(),
        },
        cx,
    );
    stage.keeper.reply(
        asked,
        Ok(Reply::ComparedForReview(vec![CreditedFile {
            file: file("a.txt", FileStatus::Modified),
            by: vec![session_id('a')],
        }])),
    );
    read_versions_of(stage, from, Some(to), ("a.txt", "one\n", "two\n"), cx);
}

fn wait_for_the_offer(stage: &Stage, cx: &mut VisualTestContext) -> ChangeShown {
    wait_until(cx, "the offer", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.flow_offer().is_some())
    });
    flow_offer(stage, cx).expect("waited for it")
}

#[gpui_kit::test]
fn follow_is_on_until_the_user_turns_it_off_with_its_control(cx: &mut TestAppContext) {
    let t = turn_id('t');
    let (mut stage, mut cx) = open(vec![workspace('1', "feature"), workspace('2', "other")], cx);
    show_changes(&mut stage, &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);

    focus_by_the_bar(&stage, &mut cx);
    press("shift-f", &mut cx);
    assert_eq!(following(&stage, &cx), Following::Off);
    press("shift-f", &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);
    click("flow-follow", &mut cx);
    assert_eq!(following(&stage, &cx), Following::Off);
    click("flow-follow", &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);

    press("shift-t", &mut cx);
    stage.keeper.set_turn(running_turn(
        t.clone(),
        session_id('a'),
        1,
        Some(&s0()),
        Vec::new(),
    ));
    stage.keeper.tell_agents(agents_of(vec![session_a_on(&t)]));
    wait_until(&mut cx, "Flow, where a turn's start leads", |cx| {
        view(&stage, cx) == View::Flow
    });
    for number in 1..=30 {
        stage.keeper.tell(step_fact(
            session_id('a'),
            t.clone(),
            number,
            alternating(number),
        ));
    }
    wait_for_at(&stage, &t, 30, &mut cx);
    stage.keeper.tell(turn_ended(&t, Some(s2()), Some(s1())));
    wait_for_the_offer(&stage, &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);

    stage
        .models
        .workspaces
        .update(&mut cx, |list, cx| list.select(&id('2'), cx));
    let second = yaar_link::Changes {
        workspace: id('2'),
        ..since_main(Vec::new(), vec![file("a.rs", FileStatus::Modified)])
    };
    stage.keeper.show(&second, |_| texts("a\n", "b\n"), &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);
    stage
        .models
        .workspaces
        .update(&mut cx, |list, cx| list.select(&id('1'), cx));
    let first = since_main(Vec::new(), vec![file("a.rs", FileStatus::Modified)]);
    stage.keeper.show(&first, |_| texts("a\n", "b\n"), &mut cx);
    cx.run_until_parked();
    assert_eq!(following(&stage, &cx), Following::On);
}

#[gpui_kit::test]
fn with_follow_on_flow_moves_to_each_step_as_it_comes_a_file_read_as_a_file_written(
    cx: &mut TestAppContext,
) {
    let t = turn_id('t');
    let (mut stage, mut cx) = at_work_in_flow(Vec::new(), cx);
    assert_eq!(following(&stage, &cx), Following::On);
    let mut furthest = 0;
    for number in 1..=30 {
        tell_step(&mut stage, &t, number, &mut cx);
        wait_for_at(&stage, &t, number, &mut cx);
        let row = wait_for_its_row_in_view(&stage, &mut cx);
        assert_eq!(row, entry_of(&stage, &t, number, &mut cx));
        assert!(
            drawn(&format!("flow-entry-{row}"), &mut cx).is_some(),
            "step {number} is drawn"
        );
        furthest = furthest.max(in_view(&stage, &mut cx).end);
    }
    assert!(furthest > 20, "the list scrolled to the steps: {furthest}");
    assert!(
        drawn("flow-entry-0", &mut cx).is_none(),
        "the turn's heading scrolled out of view"
    );
}

#[gpui_kit::test]
fn with_follow_off_a_step_leaves_flow_where_it_was(cx: &mut TestAppContext) {
    let t = turn_id('t');
    let (mut stage, mut cx) = at_work_in_flow(Vec::new(), cx);
    tell_step(&mut stage, &t, 1, &mut cx);
    wait_for_at(&stage, &t, 1, &mut cx);
    press("shift-f", &mut cx);
    assert_eq!(following(&stage, &cx), Following::Off);
    // The barrier is the step in the timeline: it was told, and Flow is not at it.
    tell_step(&mut stage, &t, 2, &mut cx);
    assert_eq!(flow_at(&stage, &cx), Some(at(&t, 1)));
    assert_eq!(following(&stage, &cx), Following::Off);
    press("shift-f", &mut cx);
    tell_step(&mut stage, &t, 3, &mut cx);
    wait_for_at(&stage, &t, 3, &mut cx);
}

#[gpui_kit::test]
fn in_the_live_diff_flow_moves_to_a_file_the_turn_read_as_to_one_it_wrote(cx: &mut TestAppContext) {
    let t = turn_id('t');
    let (mut stage, mut cx) = at_work_in_flow(Vec::new(), cx);
    let row_of = |rows: &[PaneRow], wanted: &str| {
        rows.iter().position(|row| match row {
            PaneRow::File { path, .. } | PaneRow::Read { path } => path == &PathBuf::from(wanted),
            _ => false,
        })
    };
    press("shift-l", &mut cx);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.flow_form()),
        FlowForm::LiveDiff
    );
    compared(&mut stage, &s0(), None, Vec::new(), &mut cx);

    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 1, write("a.txt")));
    compared(
        &mut stage,
        &s0(),
        None,
        vec![file("a.txt", FileStatus::Added)],
        &mut cx,
    );
    // Long enough that the files read after it are out of view until Flow goes there.
    let long: String = (0..60).map(|line| format!("line {line}\n")).collect();
    read_versions_of(&mut stage, &s0(), None, ("a.txt", "", &long), &mut cx);
    wait_until(&mut cx, "the heading of a.txt", |cx| {
        let (rows, _) = live(&stage, cx);
        row_of(&rows, "a.txt").is_some()
    });
    wait_for_at(&stage, &t, 1, &mut cx);
    let heading = wait_for_its_row_in_view(&stage, &mut cx);
    assert_eq!(
        Some(heading),
        row_of(&live(&stage, &cx).0, "a.txt"),
        "Flow is at the heading of the file the turn wrote"
    );

    tell_the_step(&mut stage, &t, 2, read("a.txt"), &mut cx);
    wait_for_at(&stage, &t, 2, &mut cx);
    assert_eq!(wait_for_its_row_in_view(&stage, &mut cx), heading);

    tell_the_step(&mut stage, &t, 3, read("z.txt"), &mut cx);
    wait_for_at(&stage, &t, 3, &mut cx);
    let (rows, at_row) = live(&stage, &cx);
    let z = row_of(&rows, "z.txt").expect("z.txt is a row of the live diff");
    assert_eq!(
        rows.last(),
        Some(&PaneRow::Read {
            path: "z.txt".into()
        })
    );
    assert_eq!(at_row, Some(z));
    assert!(
        z > 60,
        "the row of z.txt is below the long diff of a.txt: {z}"
    );
    assert_eq!(wait_for_its_row_in_view(&stage, &mut cx), z);
    assert!(drawn(&format!("flow-read-{z}"), &mut cx).is_some());
    assert_eq!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.activity().cloned()),
        None
    );

    tell_the_step(&mut stage, &t, 4, read("a.txt"), &mut cx);
    wait_for_at(&stage, &t, 4, &mut cx);
    assert_eq!(wait_for_its_row_in_view(&stage, &mut cx), heading);
    assert!(drawn(&format!("flow-read-{z}"), &mut cx).is_none());

    // Nothing was asked for the three reads: the next request is the list the write of b.txt asks
    // for.
    tell_the_step(&mut stage, &t, 5, write("b.txt"), &mut cx);
    compared(
        &mut stage,
        &s0(),
        None,
        vec![
            file("a.txt", FileStatus::Added),
            file("b.txt", FileStatus::Added),
        ],
        &mut cx,
    );
    read_versions_of(&mut stage, &s0(), None, ("b.txt", "", "new b\n"), &mut cx);
    wait_until(&mut cx, "the heading of b.txt", |cx| {
        row_of(&live(&stage, cx).0, "b.txt").is_some()
    });
    wait_for_at(&stage, &t, 5, &mut cx);
    let b = wait_for_its_row_in_view(&stage, &mut cx);
    let (rows, _) = live(&stage, &cx);
    assert_eq!(Some(b), row_of(&rows, "b.txt"));
    assert_eq!(
        rows.last(),
        Some(&PaneRow::Read {
            path: "z.txt".into()
        })
    );

    tell_the_step(&mut stage, &t, 6, command("make"), &mut cx);
    wait_for_at(&stage, &t, 6, &mut cx);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.flow_at_row()),
        None
    );
    assert_eq!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.activity().cloned()),
        Some(at(&t, 6))
    );
    assert!(drawn("flow-activity", &mut cx).is_some());

    press("shift-l", &mut cx);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.flow_form()),
        FlowForm::Timeline
    );
    tell_the_step(&mut stage, &t, 7, read("z.txt"), &mut cx);
    wait_for_at(&stage, &t, 7, &mut cx);
    let row = wait_for_its_row_in_view(&stage, &mut cx);
    assert_eq!(row, entry_of(&stage, &t, 7, &mut cx));
}

#[gpui_kit::test]
fn the_line_above_the_live_diff_names_the_step_flow_is_at_the_moment_shift_g_puts_it_there(
    cx: &mut TestAppContext,
) {
    let t = turn_id('t');
    let (mut stage, mut cx) = at_work_in_flow(Vec::new(), cx);
    let activity = |stage: &Stage, cx: &VisualTestContext| {
        stage.pane.read_with(cx, |pane, _| pane.activity().cloned())
    };
    press("shift-l", &mut cx);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.flow_form()),
        FlowForm::LiveDiff
    );
    compared(&mut stage, &s0(), None, Vec::new(), &mut cx);
    tell_the_step(&mut stage, &t, 1, read("z.txt"), &mut cx);
    wait_for_at(&stage, &t, 1, &mut cx);
    assert_eq!(activity(&stage, &cx), None);

    press("j", &mut cx);
    assert_eq!(following(&stage, &cx), Following::Paused { missed: 0 });
    tell_the_step(&mut stage, &t, 2, command("make"), &mut cx);
    assert_eq!(following(&stage, &cx), Following::Paused { missed: 1 });
    assert_eq!(flow_at(&stage, &cx), Some(at(&t, 1)));
    assert_eq!(activity(&stage, &cx), None);
    assert!(drawn("flow-activity", &mut cx).is_none());

    press("shift-g", &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);
    assert_eq!(flow_at(&stage, &cx), Some(at(&t, 2)));
    assert_eq!(activity(&stage, &cx), Some(at(&t, 2)));
    assert!(drawn("flow-activity", &mut cx).is_some());

    press("j", &mut cx);
    assert_eq!(following(&stage, &cx), Following::Paused { missed: 0 });
    tell_the_step(&mut stage, &t, 3, read("y.txt"), &mut cx);
    assert_eq!(activity(&stage, &cx), Some(at(&t, 2)));
    press("shift-g", &mut cx);
    assert_eq!(flow_at(&stage, &cx), Some(at(&t, 3)));
    assert_eq!(activity(&stage, &cx), None);
    assert!(drawn("flow-activity", &mut cx).is_none());
    assert!(live(&stage, &cx).1.is_some());
    nothing_more_was_asked(&mut stage, &mut cx);
}

#[gpui_kit::test]
fn a_step_of_the_turn_before_is_not_named_above_the_live_diff_of_the_turn_that_started(
    cx: &mut TestAppContext,
) {
    let (t, u) = (turn_id('t'), turn_id('u'));
    let (mut stage, mut cx) = at_work_in_flow(Vec::new(), cx);
    press("shift-l", &mut cx);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.flow_form()),
        FlowForm::LiveDiff
    );
    compared(&mut stage, &s0(), None, Vec::new(), &mut cx);

    tell_the_step(&mut stage, &t, 1, command("make"), &mut cx);
    wait_for_at(&stage, &t, 1, &mut cx);
    let activity = |stage: &Stage, cx: &VisualTestContext| {
        stage.pane.read_with(cx, |pane, _| pane.activity().cloned())
    };
    assert_eq!(activity(&stage, &cx), Some(at(&t, 1)));
    assert!(drawn("flow-activity", &mut cx).is_some());

    // The next turn starts, and the live diff shows it before a step is told. Follow keeps its
    // place in the turn before, which the new diff has no row for: no activity is named.
    stage.keeper.set_turn(running_turn(
        u.clone(),
        session_id('a'),
        2,
        Some(&s1()),
        Vec::new(),
    ));
    stage.keeper.tell_agents(agents_of(vec![session_a_on(&u)]));
    compared(&mut stage, &s1(), None, Vec::new(), &mut cx);
    cx.run_until_parked();
    let shown = stage.pane.read_with(&cx, |pane, _| {
        pane.live_change().map(|change| change.turn.clone())
    });
    assert_eq!(shown, Some(u.clone()));
    assert_eq!(flow_at(&stage, &cx), Some(at(&t, 1)));
    assert_eq!(activity(&stage, &cx), None);
    assert!(drawn("flow-activity", &mut cx).is_none());

    tell_the_step(&mut stage, &u, 1, command("ls"), &mut cx);
    wait_for_at(&stage, &u, 1, &mut cx);
    assert_eq!(activity(&stage, &cx), Some(at(&u, 1)));
    assert!(drawn("flow-activity", &mut cx).is_some());
}

#[gpui_kit::test]
fn a_scroll_a_click_or_a_key_pauses_follow_the_missed_steps_are_counted_and_the_offer_resumes_it(
    cx: &mut TestAppContext,
) {
    let t = turn_id('t');
    let (mut stage, mut cx) = at_work_in_flow((1..=24).map(alternating).collect(), cx);
    for number in 25..=27 {
        tell_step(&mut stage, &t, number, &mut cx);
    }
    wait_for_at(&stage, &t, 27, &mut cx);
    wait_for_its_row_in_view(&stage, &mut cx);
    assert!(drawn("flow-follow-paused", &mut cx).is_none());

    wheel(&stage, 300., &mut cx);
    assert_eq!(following(&stage, &cx), Following::Paused { missed: 0 });
    assert!(drawn("flow-follow-paused", &mut cx).is_some());
    assert!(drawn("flow-follow-resume", &mut cx).is_some());

    for number in 28..=30 {
        tell_step(&mut stage, &t, number, &mut cx);
    }
    assert_eq!(following(&stage, &cx), Following::Paused { missed: 3 });
    assert_eq!(flow_at(&stage, &cx), Some(at(&t, 27)));
    assert!(
        drawn(
            "flow-follow-paused-text:Follow paused: 3 steps since",
            &mut cx
        )
        .is_some(),
        "the line names the count"
    );
    wheel(&stage, 50., &mut cx);
    assert_eq!(following(&stage, &cx), Following::Paused { missed: 3 });

    assert!(
        !in_view(&stage, &mut cx).contains(&entry_of(&stage, &t, 30, &mut cx)),
        "the newest step is out of view before resuming"
    );
    click("flow-follow-resume", &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);
    assert_eq!(flow_at(&stage, &cx), Some(at(&t, 30)));
    assert_eq!(
        wait_for_its_row_in_view(&stage, &mut cx),
        entry_of(&stage, &t, 30, &mut cx)
    );
    assert!(drawn("flow-follow-paused", &mut cx).is_none());

    click_an_entry(&stage, &mut cx);
    assert_eq!(following(&stage, &cx), Following::Paused { missed: 0 });
    tell_step(&mut stage, &t, 31, &mut cx);
    assert_eq!(following(&stage, &cx), Following::Paused { missed: 1 });
    assert!(
        drawn(
            "flow-follow-paused-text:Follow paused: 1 step since",
            &mut cx
        )
        .is_some(),
        "one step is no plural"
    );
    press("shift-g", &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);
    assert_eq!(flow_at(&stage, &cx), Some(at(&t, 31)));
    assert_eq!(
        wait_for_its_row_in_view(&stage, &mut cx),
        entry_of(&stage, &t, 31, &mut cx)
    );

    press("j", &mut cx);
    assert_eq!(following(&stage, &cx), Following::Paused { missed: 0 });
    tell_step(&mut stage, &t, 32, &mut cx);
    assert_eq!(following(&stage, &cx), Following::Paused { missed: 1 });
    press("shift-g", &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);
    assert_eq!(flow_at(&stage, &cx), Some(at(&t, 32)));
    assert_eq!(
        wait_for_its_row_in_view(&stage, &mut cx),
        entry_of(&stage, &t, 32, &mut cx)
    );
}

#[gpui_kit::test]
fn the_controls_of_the_views_and_of_follow_do_not_pause_follow_and_nothing_pauses_it_with_no_turn_running(
    cx: &mut TestAppContext,
) {
    let t = turn_id('t');
    let (mut stage, mut cx) = at_work_in_flow(vec![read("a.txt"), write("b.txt")], cx);

    stage.keeper.tell_agents(agents_of(vec![session_a_idle()]));
    tell_step(&mut stage, &t, 3, &mut cx);
    click_an_entry(&stage, &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);
    press("j", &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);
    wheel(&stage, 20., &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);

    stage.keeper.tell_agents(agents_of(vec![session_a_on(&t)]));
    tell_step(&mut stage, &t, 4, &mut cx);

    press("shift-l", &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);
    press("shift-l", &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);
    press("f", &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);
    press("f", &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);
    click("review-bar", &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);
    click("review-view-changes", &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);
    click("review-view-flow", &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);
    click("flow-form-live", &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);
    click("flow-form-timeline", &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);
    press("shift-g", &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);
    press("shift-f", &mut cx);
    assert_eq!(following(&stage, &cx), Following::Off);
    press("shift-f", &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);
    click("flow-follow", &mut cx);
    click("flow-follow", &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);
}

#[gpui_kit::test]
fn a_key_no_action_takes_pauses_follow_too(cx: &mut TestAppContext) {
    let t = turn_id('t');
    let (mut stage, mut cx) = at_work_in_flow(vec![read("a.txt")], cx);
    // `z` is bound to nothing: it reaches the pane as a key, which is the reviewer typing.
    press("z", &mut cx);
    assert_eq!(following(&stage, &cx), Following::Paused { missed: 0 });
    tell_step(&mut stage, &t, 2, &mut cx);
    assert_eq!(following(&stage, &cx), Following::Paused { missed: 1 });
}

#[gpui_kit::test]
fn a_step_of_another_workspace_is_not_counted_and_a_pause_ends_with_the_workspace(
    cx: &mut TestAppContext,
) {
    let t = turn_id('t');
    let (mut stage, mut cx) = at_work_in_flow(vec![read("a.txt")], cx);
    wheel_once(&stage, &mut cx);
    assert_eq!(following(&stage, &cx), Following::Paused { missed: 0 });
    let mut elsewhere = step_fact(session_id('a'), t.clone(), 9, read("elsewhere.txt"));
    if let Fact::Step { workspace, .. } = &mut elsewhere {
        *workspace = id('2');
    }
    stage.keeper.tell(elsewhere);
    tell_step(&mut stage, &t, 2, &mut cx);
    assert_eq!(following(&stage, &cx), Following::Paused { missed: 1 });

    stage
        .models
        .workspaces
        .update(&mut cx, |list, cx| list.select(&id('2'), cx));
    let second = yaar_link::Changes {
        workspace: id('2'),
        ..since_main(Vec::new(), vec![file("a.rs", FileStatus::Modified)])
    };
    stage.keeper.show(&second, |_| texts("a\n", "b\n"), &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);
}

fn wheel_once(stage: &Stage, cx: &mut VisualTestContext) {
    wheel(stage, 10., cx);
}

#[gpui_kit::test]
fn resuming_says_when_nothing_is_paused_or_follow_is_off_and_turning_it_off_ends_a_pause(
    cx: &mut TestAppContext,
) {
    let t = turn_id('t');
    let (mut stage, mut cx) = at_work_in_flow(vec![read("a.txt")], cx);
    assert_eq!(notice(&stage, &cx), None);
    press("shift-g", &mut cx);
    assert_eq!(notice(&stage, &cx).as_deref(), Some("Follow is not paused"));
    press("shift-f", &mut cx);
    assert_eq!(following(&stage, &cx), Following::Off);
    assert_eq!(notice(&stage, &cx), None);
    press("shift-g", &mut cx);
    assert_eq!(notice(&stage, &cx).as_deref(), Some("Follow is off"));
    assert_eq!(following(&stage, &cx), Following::Off);
    press("shift-f", &mut cx);

    press("z", &mut cx);
    assert_eq!(following(&stage, &cx), Following::Paused { missed: 0 });
    tell_step(&mut stage, &t, 2, &mut cx);
    press("shift-f", &mut cx);
    assert_eq!(following(&stage, &cx), Following::Off);
    press("shift-f", &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);

    press("shift-o", &mut cx);
    assert!(notice(&stage, &cx).is_some());
    assert_eq!(view(&stage, &cx), View::Flow);
}

#[gpui_kit::test]
fn with_follow_on_a_message_of_the_user_to_the_agent_switches_the_pane_to_flow_and_with_follow_off_it_does_not(
    cx: &mut TestAppContext,
) {
    let (t0, t1, t2, t3) = (turn_id('0'), turn_id('1'), turn_id('2'), turn_id('3'));
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature")],
        |keeper| keeper.set_agents(vec![agents_of(vec![session_a_idle()])]),
        cx,
    );
    show_changes(&mut stage, &mut cx);
    assert_eq!(view(&stage, &cx), View::Changes);
    assert_eq!(following(&stage, &cx), Following::On);

    // A turn the CLI started itself, a task it finished or a hook it ran, is no message of the
    // user: Changes stays on screen. A step of that turn, told after, is the barrier.
    stage.keeper.set_turn(running_turn(
        t0.clone(),
        session_id('a'),
        1,
        Some(&s0()),
        Vec::new(),
    ));
    stage
        .keeper
        .tell_agents(agents_of(vec![self_started(session_a_idle(), t0.clone())]));
    stage
        .keeper
        .tell(step_fact(session_id('a'), t0.clone(), 1, read("x.txt")));
    wait_for_step(&stage, &t0, 1, &mut cx);
    assert_eq!(view(&stage, &cx), View::Changes);
    assert_eq!(following(&stage, &cx), Following::On);

    for turn in [&t1, &t2, &t3] {
        stage.keeper.set_turn(running_turn(
            turn.clone(),
            session_id('a'),
            1,
            Some(&s0()),
            Vec::new(),
        ));
    }
    stage.keeper.tell_agents(agents_of(vec![session_a_on(&t1)]));
    wait_until(&mut cx, "Flow after the message", |cx| {
        view(&stage, cx) == View::Flow
    });
    assert_eq!(following(&stage, &cx), Following::On);

    focus_by_the_bar(&stage, &mut cx);
    press("f", &mut cx);
    assert_eq!(view(&stage, &cx), View::Changes);
    assert_eq!(following(&stage, &cx), Following::On);
    click("review-row-1", &mut cx);
    assert_eq!(following(&stage, &cx), Following::Paused { missed: 0 });
    stage.keeper.tell_agents(agents_of(vec![session_a_on(&t2)]));
    wait_until(&mut cx, "Flow after the second message", |cx| {
        view(&stage, cx) == View::Flow
    });
    assert_eq!(following(&stage, &cx), Following::On);

    press("f", &mut cx);
    press("shift-f", &mut cx);
    assert_eq!(view(&stage, &cx), View::Changes);
    assert_eq!(following(&stage, &cx), Following::Off);
    stage.keeper.tell_agents(agents_of(vec![session_a_on(&t3)]));
    stage
        .keeper
        .tell(step_fact(session_id('a'), t3.clone(), 1, read("x.txt")));
    wait_for_step(&stage, &t3, 1, &mut cx);
    assert_eq!(view(&stage, &cx), View::Changes);
    assert_eq!(following(&stage, &cx), Following::Off);
}

#[gpui_kit::test]
fn a_pane_that_connects_while_a_turn_runs_stays_in_changes_and_so_does_a_turn_that_only_changes_state(
    cx: &mut TestAppContext,
) {
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
                Vec::new(),
            ));
        },
        cx,
    );
    show_changes(&mut stage, &mut cx);
    let waiting = Session {
        state: SessionState::WaitsForPermission,
        ..session_a_on(&t)
    };
    stage.keeper.tell_agents(agents_of(vec![waiting]));
    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 1, read("x.txt")));
    wait_for_step(&stage, &t, 1, &mut cx);
    assert_eq!(view(&stage, &cx), View::Changes);
    assert_eq!(following(&stage, &cx), Following::On);
}

#[gpui_kit::test]
fn agents_read_after_the_changes_are_shown_start_nothing_and_a_turn_that_only_changes_state_stays_in_changes(
    cx: &mut TestAppContext,
) {
    let t = turn_id('t');
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature")],
        |keeper| {
            keeper.set_agents(vec![agents_of(vec![session_a_on(&turn_id('t'))])]);
            // A Keeper slow with its list of the agents: the changes are on screen long before the
            // turn running in it is known.
            keeper.hold_agents();
        },
        cx,
    );
    wait_until(&mut cx, "the question for the agents", |_| {
        stage.keeper.held_agents() == 1
    });
    show_changes(&mut stage, &mut cx);
    stage.keeper.release_agents(1);
    wait_until(&mut cx, "the agents of the workspace", |cx| {
        stage
            .models
            .agents
            .read_with(cx, |agents, _| agents.of(&id('1')).is_some())
    });
    let waiting = Session {
        state: SessionState::WaitsForPermission,
        ..session_a_on(&t)
    };
    stage.keeper.tell_agents(agents_of(vec![waiting]));
    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 1, read("x.txt")));
    wait_for_step(&stage, &t, 1, &mut cx);
    assert_eq!(view(&stage, &cx), View::Changes);
    assert_eq!(following(&stage, &cx), Following::On);
}

#[gpui_kit::test]
fn a_workspace_that_comes_on_screen_with_a_turn_running_starts_nothing(cx: &mut TestAppContext) {
    let t = turn_id('t');
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature"), workspace('2', "other")],
        |keeper| {
            keeper.set_agents(vec![agents_of(vec![session_a_on(&turn_id('t'))])]);
            keeper.set_turn(running_turn(
                turn_id('t'),
                session_id('a'),
                1,
                Some(&s0()),
                Vec::new(),
            ));
        },
        cx,
    );
    show_changes(&mut stage, &mut cx);
    stage
        .models
        .workspaces
        .update(&mut cx, |list, cx| list.select(&id('2'), cx));
    let second = yaar_link::Changes {
        workspace: id('2'),
        ..since_main(Vec::new(), vec![file("a.rs", FileStatus::Modified)])
    };
    stage.keeper.show(&second, |_| texts("a\n", "b\n"), &mut cx);
    stage
        .models
        .workspaces
        .update(&mut cx, |list, cx| list.select(&id('1'), cx));
    let first = since_main(Vec::new(), vec![file("a.rs", FileStatus::Modified)]);
    stage.keeper.show(&first, |_| texts("a\n", "b\n"), &mut cx);
    let waiting = Session {
        state: SessionState::WaitsForPermission,
        ..session_a_on(&t)
    };
    stage.keeper.tell_agents(agents_of(vec![waiting]));
    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 1, read("x.txt")));
    wait_for_step(&stage, &t, 1, &mut cx);
    assert_eq!(view(&stage, &cx), View::Changes);
}

#[gpui_kit::test]
fn when_a_turn_ends_while_flow_is_on_screen_the_pane_stays_in_flow_and_offers_what_changed_since_the_last_review(
    cx: &mut TestAppContext,
) {
    let t = turn_id('t');
    let (mut stage, mut cx) = at_work_in_flow(vec![read("a.txt")], cx);

    stage.keeper.tell(turn_ended(&t, Some(s2()), Some(s1())));
    the_turn_end_selects(&mut stage, &s1(), &s2(), &mut cx);
    let since = ChangeShown::Since {
        from: Some(s1()),
        to: s2(),
    };
    assert_eq!(wait_for_the_offer(&stage, &mut cx), since);
    assert_eq!(view(&stage, &cx), View::Flow);
    assert!(drawn("flow-offer", &mut cx).is_some());
    assert!(drawn("flow-offer-take", &mut cx).is_some());

    stage.keeper.tell_agents(agents_of(vec![session_a_idle()]));
    tell_a_step_after(&mut stage, &t, 2, &mut cx);
    assert_eq!(flow_offer(&stage, &cx), Some(since.clone()));

    // Taking the offer shows Changes with that change and asks nothing more: the barrier is the
    // changes of a workspace put on screen after it.
    click("flow-offer-take", &mut cx);
    assert_eq!(view(&stage, &cx), View::Changes);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.change_shown()),
        since
    );
    assert_eq!(flow_offer(&stage, &cx), None);
    nothing_more_was_asked(&mut stage, &mut cx);
}

fn tell_a_step_after(stage: &mut Stage, t: &TurnId, number: u32, cx: &mut VisualTestContext) {
    stage.keeper.tell(step_fact(
        session_id('a'),
        t.clone(),
        number,
        read("late.txt"),
    ));
    wait_for_step(stage, t, number, cx);
}

fn nothing_more_was_asked(stage: &mut Stage, cx: &mut VisualTestContext) {
    stage
        .models
        .workspaces
        .update(cx, |list, cx| list.select(&id('2'), cx));
    let second = yaar_link::Changes {
        workspace: id('2'),
        ..since_main(Vec::new(), Vec::new())
    };
    stage.keeper.show(&second, |_| texts("a\n", "b\n"), cx);
}

#[gpui_kit::test]
fn the_offer_is_taken_with_shift_o_and_a_turn_end_with_no_since_counts_from_the_history_start(
    cx: &mut TestAppContext,
) {
    let t = turn_id('t');
    let (mut stage, mut cx) = at_work_in_flow_with(vec![read("a.txt")], Some(history_start()), cx);
    stage.keeper.tell(turn_ended(&t, Some(s2()), None));
    the_turn_end_selects(&mut stage, &history_start(), &s2(), &mut cx);
    let since = ChangeShown::Since {
        from: Some(history_start()),
        to: s2(),
    };
    assert_eq!(wait_for_the_offer(&stage, &mut cx), since);
    assert_eq!(view(&stage, &cx), View::Flow);

    press("shift-o", &mut cx);
    assert_eq!(view(&stage, &cx), View::Changes);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.change_shown()),
        since
    );
    assert_eq!(flow_offer(&stage, &cx), None);
    nothing_more_was_asked(&mut stage, &mut cx);
}

#[gpui_kit::test]
fn a_pane_that_keeps_the_selection_offers_in_flow_and_asks_for_the_change_when_the_offer_is_taken(
    cx: &mut TestAppContext,
) {
    let t = turn_id('t');
    let (mut stage, mut cx) = at_work_in_flow(vec![read("a.txt")], cx);
    press("shift-t", &mut cx);
    assert!(stage.pane.read_with(&cx, |pane, _| pane.keeps_selection()));

    stage.keeper.tell(turn_ended(&t, Some(s2()), Some(s1())));
    let since = ChangeShown::Since {
        from: Some(s1()),
        to: s2(),
    };
    assert_eq!(wait_for_the_offer(&stage, &mut cx), since);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.offer().cloned()),
        Some(since.clone())
    );
    assert_eq!(view(&stage, &cx), View::Flow);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.change_shown()),
        ChangeShown::Comparison
    );

    click("flow-offer-take", &mut cx);
    the_turn_end_selects(&mut stage, &s1(), &s2(), &mut cx);
    assert_eq!(view(&stage, &cx), View::Changes);
    wait_until(&mut cx, "the change selected", |cx| {
        stage.pane.read_with(cx, |pane, _| pane.change_shown()) == since
    });
    assert_eq!(flow_offer(&stage, &cx), None);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.offer().cloned()),
        None
    );
}

#[gpui_kit::test]
fn with_changes_on_screen_at_a_turn_end_nothing_is_offered_in_flow(cx: &mut TestAppContext) {
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
                Vec::new(),
            ));
        },
        cx,
    );
    show_changes(&mut stage, &mut cx);
    assert_eq!(view(&stage, &cx), View::Changes);
    stage.keeper.tell(turn_ended(&t, Some(s2()), Some(s1())));
    // The barrier is the request Changes makes for the selection it takes.
    the_turn_end_selects(&mut stage, &s1(), &s2(), &mut cx);
    cx.run_until_parked();
    assert_eq!(flow_offer(&stage, &cx), None);
    focus_by_the_bar(&stage, &mut cx);
    press("f", &mut cx);
    assert_eq!(view(&stage, &cx), View::Flow);
    assert_eq!(flow_offer(&stage, &cx), None);
    assert!(drawn("flow-offer", &mut cx).is_none());
}

#[gpui_kit::test]
fn a_turn_end_with_no_state_offers_nothing(cx: &mut TestAppContext) {
    let t = turn_id('t');
    let (mut stage, mut cx) = at_work_in_flow(vec![read("a.txt")], cx);
    stage.keeper.tell(turn_ended(&t, None, None));
    wait_until(&mut cx, "the end in the timeline", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.timeline()
                .iter()
                .any(|entry| matches!(entry, TimelineEntry::End { .. }))
        })
    });
    tell_a_step_after(&mut stage, &t, 2, &mut cx);
    assert_eq!(flow_offer(&stage, &cx), None);
    assert_eq!(view(&stage, &cx), View::Flow);
}

#[gpui_kit::test]
fn the_offer_goes_when_changes_is_shown_when_another_turn_starts_and_when_the_workspace_changes(
    cx: &mut TestAppContext,
) {
    let t = turn_id('t');
    let t2 = turn_id('2');
    let (mut stage, mut cx) = at_work_in_flow(vec![read("a.txt")], cx);
    // The selection is kept, so that no turn end asks anything of Changes.
    press("shift-t", &mut cx);

    stage.keeper.tell(turn_ended(&t, Some(s2()), Some(s1())));
    wait_for_the_offer(&stage, &mut cx);
    press("f", &mut cx);
    assert_eq!(view(&stage, &cx), View::Changes);
    assert_eq!(flow_offer(&stage, &cx), None);
    press("f", &mut cx);
    assert_eq!(flow_offer(&stage, &cx), None);

    stage.keeper.tell(turn_ended(&t, Some(s2()), Some(s1())));
    wait_for_the_offer(&stage, &mut cx);
    stage.keeper.set_turn(running_turn(
        t2.clone(),
        session_id('a'),
        2,
        Some(&s2()),
        Vec::new(),
    ));
    stage.keeper.tell_agents(agents_of(vec![session_a_on(&t2)]));
    wait_until(&mut cx, "the offer to go", |cx| {
        flow_offer(&stage, cx).is_none()
    });

    stage.keeper.tell(turn_ended(&t2, Some(s2()), Some(s1())));
    wait_for_the_offer(&stage, &mut cx);
    stage
        .models
        .workspaces
        .update(&mut cx, |list, cx| list.select(&id('2'), cx));
    let second = yaar_link::Changes {
        workspace: id('2'),
        ..since_main(Vec::new(), vec![file("a.rs", FileStatus::Modified)])
    };
    stage.keeper.show(&second, |_| texts("a\n", "b\n"), &mut cx);
    assert_eq!(flow_offer(&stage, &cx), None);
    stage
        .models
        .workspaces
        .update(&mut cx, |list, cx| list.select(&id('1'), cx));
    let first = since_main(Vec::new(), vec![file("a.rs", FileStatus::Modified)]);
    stage.keeper.show(&first, |_| texts("a\n", "b\n"), &mut cx);
    cx.run_until_parked();
    assert_eq!(flow_offer(&stage, &cx), None);
}

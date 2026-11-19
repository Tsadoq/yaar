//! Component test: follow and the offer at a turn end, on GPUI's test platform. Keys, clicks
//! and the wheel go in; the pane is read back through the model it shows. The test plays the
//! Keeper on the other end of a socket pair: it sets the agents and the turns the Keeper
//! would hold, tells the facts the Keeper tells, and answers what the pane asks.
//!
//! A negative clause ("does not switch", "asks nothing") is read after a barrier: something
//! the test causes after it, through the same path, whose arrival shows that everything
//! before it arrived. A fact the Keeper tells reaches the pane by a thread of its own, so
//! every wait is a wait for a fact, with a deadline.

mod common;

use std::ops::Range;
use std::path::PathBuf;

use gpui_kit::{
    AppContext, Modifiers, ScrollDelta, ScrollEvent, TestAppContext, VisualTestContext, point, px,
};
use yaar_link::{
    ChangedFile, Fact, FileStatus, FlowForm, History, Reply, Request, Session, SessionState, Step,
    TurnId,
};
use yaar_review_pane::{ChangeShown, FlowAt, Following, PaneRow, TimelineEntry, View};

use common::{
    Stage, agents_of, drawn, file, id, open, open_staged, review_id, review_with, reviews_of,
    running_turn, session, session_id, since_main, step_fact, texts, turn_id, wait_for_files,
    wait_until, working_on, workspace,
};

/// The state the turn counts from.
fn s0() -> String {
    "5".repeat(40)
}

/// The state the reviewer last marked reviewed.
fn s1() -> String {
    "6".repeat(40)
}

/// The state recorded at the end of the turn.
fn s2() -> String {
    "7".repeat(40)
}

/// Where the history of the workspace starts, which is not the commit the changes are
/// counted from, so that a count from the wrong one shows.
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

/// Step `number` of the alternation the tests use: a read of a file of its own for an odd
/// number, a write of one for an even number.
fn alternating(number: u32) -> Step {
    match number % 2 {
        1 => read(&format!("r{number}.txt")),
        _ => write(&format!("w{number}.txt")),
    }
}

/// Session `a`, which works on `turn` and holds the review of the tests.
fn session_a_on(turn: &TurnId) -> Session {
    working_on(
        session('a', "Claude Code 1", SessionState::Works, Some(review_id())),
        turn.clone(),
    )
}

/// Session `a` between turns.
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

/// A click on the bar, away from every control of it: the pane has the focus afterwards,
/// and nothing was read or written by the reviewer.
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

/// The rows of the live diff and the row Flow is at in it.
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

/// Keeps the app running until the timeline holds step `number` of `turn`.
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

/// Keeps the app running until Flow is at step `number` of `turn`.
fn wait_for_at(stage: &Stage, turn: &TurnId, number: u32, cx: &mut VisualTestContext) {
    let wanted = at(turn, number);
    wait_until(cx, "Flow at the step", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.flow_at() == Some(&wanted))
    });
}

/// Keeps the app running until the row Flow is at is in view, and returns it.
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

/// The index in the timeline of the entry of step `number` of `turn`.
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

/// Shows the changes of one file of workspace `1`, so that what the Window asks first has
/// been answered and the requests that come after are Flow's and the turn end's.
fn show_changes(stage: &mut Stage, cx: &mut VisualTestContext) {
    let changes = since_main(Vec::new(), vec![file("a.rs", FileStatus::Modified)]);
    stage.keeper.show(&changes, |_| texts("a\n", "b\n"), cx);
    wait_for_files(&stage.pane, cx);
}

/// Turns the wheel over the entries of the timeline in view, `by` pixels (positive: up the
/// list).
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

/// A click on an entry of the timeline in view.
fn click_an_entry(stage: &Stage, cx: &mut VisualTestContext) {
    let in_view = in_view(stage, cx);
    click(&format!("flow-entry-{}", in_view.start + 1), cx);
}

/// A pane that meets session `a` working on turn `t` (read with `steps`), with Flow on
/// screen in the timeline form, the focus in the pane and follow on.
fn at_work_in_flow(steps: Vec<Step>, cx: &mut TestAppContext) -> (Stage, VisualTestContext) {
    at_work_in_flow_with(steps, None, cx)
}

/// [`at_work_in_flow`] with the history of workspace `1` starting at `start`, when given.
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

/// Tells step `number` of the alternation of turn `t` of session `a` and waits for it.
fn tell_step(stage: &mut Stage, t: &TurnId, number: u32, cx: &mut VisualTestContext) {
    stage.keeper.tell(step_fact(
        session_id('a'),
        t.clone(),
        number,
        alternating(number),
    ));
    wait_for_step(stage, t, number, cx);
}

/// Tells step `number` of turn `t` of session `a` and waits for the timeline to hold it.
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

/// Answers the next request, which has to list the files of the change from `from` to `to`,
/// with `files`.
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

/// Answers the next request, which has to read the two versions of one file, from `old` to
/// `new`.
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

/// What the Keeper tells when the turn `t` of session `a` ends in workspace `1`.
fn turn_ended(t: &TurnId, state: Option<String>, since: Option<String>) -> Fact {
    Fact::TurnEnded {
        workspace: id('1'),
        session: session_id('a'),
        turn: t.clone(),
        state,
        since,
    }
}

/// Answers what a turn end under the default setting asks for Changes: the list of what
/// changed from `from` to `to`, holding one file, and that file's two versions.
fn the_turn_end_selects(stage: &mut Stage, from: &str, to: &str, cx: &mut VisualTestContext) {
    compared(
        stage,
        from,
        Some(to),
        vec![file("a.txt", FileStatus::Modified)],
        cx,
    );
    read_versions_of(stage, from, Some(to), ("a.txt", "one\n", "two\n"), cx);
}

/// Keeps the app running until the pane offers a change in Flow, and returns it.
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

    // The key and the control in the bar turn it off and on, each as often as asked.
    focus_by_the_bar(&stage, &mut cx);
    press("shift-f", &mut cx);
    assert_eq!(following(&stage, &cx), Following::Off);
    press("shift-f", &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);
    click("flow-follow", &mut cx);
    assert_eq!(following(&stage, &cx), Following::Off);
    click("flow-follow", &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);

    // Nothing else turns it off: a turn's start, its steps, its end and another workspace
    // on screen and back leave it on. The selection is kept so that the turn end asks
    // nothing of Changes (no turn runs yet, so the key does not pause follow).
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
        // The row of the step is its entry, a read's as a write's, and it is in view: the
        // list scrolls, since thirty entries do not fit the window.
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
    // Turned on again, follow moves with the next step.
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
    // The live diff asks for the files of the turn as it comes on screen.
    press("shift-l", &mut cx);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.flow_form()),
        FlowForm::LiveDiff
    );
    compared(&mut stage, &s0(), None, Vec::new(), &mut cx);

    // A file written: Flow is at its heading once the list holds it.
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
    // A file long enough that the files read, after it, are out of view until Flow goes
    // there, and its heading is out of view when Flow is down there.
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

    // A file the turn changed, read: the same heading.
    tell_the_step(&mut stage, &t, 2, read("a.txt"), &mut cx);
    wait_for_at(&stage, &t, 2, &mut cx);
    assert_eq!(wait_for_its_row_in_view(&stage, &mut cx), heading);

    // A file the turn did not change, read: Flow is at the row of that file among the
    // files read, with its mark, and the live diff names no activity above its files.
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

    // A read of the file the turn changed takes Flow back up to its heading.
    tell_the_step(&mut stage, &t, 4, read("a.txt"), &mut cx);
    wait_for_at(&stage, &t, 4, &mut cx);
    assert_eq!(wait_for_its_row_in_view(&stage, &mut cx), heading);
    assert!(drawn(&format!("flow-read-{z}"), &mut cx).is_none());

    // Nothing was asked for the three reads: the next request is the list the write of
    // b.txt asks for. That list holds both files, and z.txt is still the last row.
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

    // A command has no row: the line above the files names it.
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

    // Back in the timeline, a second read of z.txt is at its own entry.
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
fn a_scroll_a_click_or_a_key_pauses_follow_the_missed_steps_are_counted_and_the_offer_resumes_it(
    cx: &mut TestAppContext,
) {
    let t = turn_id('t');
    // Twenty-four steps are listed when the pane meets the turn: more than the window
    // holds, so that the newest step is out of view whenever the list was scrolled up.
    let (mut stage, mut cx) = at_work_in_flow((1..=24).map(alternating).collect(), cx);
    for number in 25..=27 {
        tell_step(&mut stage, &t, number, &mut cx);
    }
    wait_for_at(&stage, &t, 27, &mut cx);
    wait_for_its_row_in_view(&stage, &mut cx);
    assert!(drawn("flow-follow-paused", &mut cx).is_none());

    // A scroll pauses follow, with nothing missed yet, and offers to resume.
    wheel(&stage, 300., &mut cx);
    assert_eq!(following(&stage, &cx), Following::Paused { missed: 0 });
    assert!(drawn("flow-follow-paused", &mut cx).is_some());
    assert!(drawn("flow-follow-resume", &mut cx).is_some());

    // The steps that come meanwhile are counted, and Flow stays where it was.
    for number in 28..=30 {
        tell_step(&mut stage, &t, number, &mut cx);
    }
    assert_eq!(following(&stage, &cx), Following::Paused { missed: 3 });
    assert_eq!(flow_at(&stage, &cx), Some(at(&t, 27)));
    // The count is said: three steps.
    assert!(
        drawn(
            "flow-follow-paused-text:Follow paused: 3 steps since",
            &mut cx
        )
        .is_some(),
        "the line names the count"
    );
    // A further turn of the wheel leaves the count as it is.
    wheel(&stage, 50., &mut cx);
    assert_eq!(following(&stage, &cx), Following::Paused { missed: 3 });

    // The offer to resume jumps to the newest step, in view.
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

    // A click on an entry pauses it; `shift-g` resumes at the newest step.
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

    // A key pauses it: the key of an action of the pane.
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

    // With no turn running a click on an entry, a key and the wheel leave follow on.
    stage.keeper.tell_agents(agents_of(vec![session_a_idle()]));
    // The barrier: a step told after the agents, which the timeline holds.
    tell_step(&mut stage, &t, 3, &mut cx);
    click_an_entry(&stage, &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);
    press("j", &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);
    wheel(&stage, 20., &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);

    // The same turn runs again: not a start (follow met it), so Flow stays as it is.
    stage.keeper.tell_agents(agents_of(vec![session_a_on(&t)]));
    tell_step(&mut stage, &t, 4, &mut cx);

    // The controls, by key and by click, in Flow and out of it: follow stays on.
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
    // `x` is bound to nothing: it reaches the pane as a key, which is the reviewer typing.
    press("x", &mut cx);
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
    // A step of workspace 2, then one of workspace 1: the barrier is the second.
    let mut elsewhere = step_fact(session_id('a'), t.clone(), 9, read("elsewhere.txt"));
    if let Fact::Step { workspace, .. } = &mut elsewhere {
        *workspace = id('2');
    }
    stage.keeper.tell(elsewhere);
    tell_step(&mut stage, &t, 2, &mut cx);
    assert_eq!(following(&stage, &cx), Following::Paused { missed: 1 });

    // Another workspace on screen: what was paused is not paused there.
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

/// A turn of the wheel over the first entry in view.
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
    // The next control that is acted on takes the saying away.
    press("shift-f", &mut cx);
    assert_eq!(following(&stage, &cx), Following::Off);
    assert_eq!(notice(&stage, &cx), None);
    press("shift-g", &mut cx);
    assert_eq!(notice(&stage, &cx).as_deref(), Some("Follow is off"));
    assert_eq!(following(&stage, &cx), Following::Off);
    press("shift-f", &mut cx);

    // Turning follow off from a pause turns it off, and on again puts it on.
    press("x", &mut cx);
    assert_eq!(following(&stage, &cx), Following::Paused { missed: 0 });
    tell_step(&mut stage, &t, 2, &mut cx);
    press("shift-f", &mut cx);
    assert_eq!(following(&stage, &cx), Following::Off);
    press("shift-f", &mut cx);
    assert_eq!(following(&stage, &cx), Following::On);

    // With nothing offered, taking the offer says so.
    press("shift-o", &mut cx);
    assert!(notice(&stage, &cx).is_some());
    assert_eq!(view(&stage, &cx), View::Flow);
}

#[gpui_kit::test]
fn with_follow_on_a_message_of_the_user_to_the_agent_switches_the_pane_to_flow_and_with_follow_off_it_does_not(
    cx: &mut TestAppContext,
) {
    let (t1, t2, t3) = (turn_id('1'), turn_id('2'), turn_id('3'));
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature")],
        |keeper| keeper.set_agents(vec![agents_of(vec![session_a_idle()])]),
        cx,
    );
    show_changes(&mut stage, &mut cx);
    assert_eq!(view(&stage, &cx), View::Changes);
    assert_eq!(following(&stage, &cx), Following::On);

    // A message: a turn follow has not met starts, and Flow is shown.
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

    // Changes again, and a click on a row of it while the turn runs pauses follow. The
    // next message ends the pause, with nothing missed, and shows Flow.
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

    // With follow off a message changes nothing: a step of the turn told after it, which
    // the timeline holds, is the barrier.
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
    // The same turn, now waiting for a permission: the session's state changed, nothing
    // started. A step of the turn told after it, which the timeline holds, is the barrier.
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
            // A Keeper that is slow with its list of the agents: the changes of the
            // workspace are on screen long before the turn running in it is known.
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
    // The session's state changes with the turn it already ran: nothing started. A step
    // told after it, which the timeline holds, is the barrier.
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
    // Away from the workspace and back to it, with its turn still running.
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
    // The turn is no message of the user, though the Keeper tells of its session after
    // the return (here, waiting for a permission): the barrier is a step told after it.
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

    // Under the default setting the turn end selects in Changes at once, and asks for it.
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

    // The Keeper tells the session idle after the turn: the offer stays.
    stage.keeper.tell_agents(agents_of(vec![session_a_idle()]));
    tell_a_step_after(&mut stage, &t, 2, &mut cx);
    assert_eq!(flow_offer(&stage, &cx), Some(since.clone()));

    // Taking it shows Changes with that change, and asks for nothing more: the barrier is
    // the changes of a workspace put on screen after it.
    click("flow-offer-take", &mut cx);
    assert_eq!(view(&stage, &cx), View::Changes);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.change_shown()),
        since
    );
    assert_eq!(flow_offer(&stage, &cx), None);
    nothing_more_was_asked(&mut stage, &mut cx);
}

/// Tells step `number` of turn `t` after the turn ended, and waits for the timeline to hold
/// it.
fn tell_a_step_after(stage: &mut Stage, t: &TurnId, number: u32, cx: &mut VisualTestContext) {
    stage.keeper.tell(step_fact(
        session_id('a'),
        t.clone(),
        number,
        read("late.txt"),
    ));
    wait_for_step(stage, t, number, cx);
}

/// Asserts that the Window asked nothing since: it is told of workspace `2` coming on
/// screen, and its changes are the next request.
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
    // The history of the workspace starts at a commit of its own, not the one the changes
    // are counted from.
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
    // At the turn end nothing is asked: the offer of Changes is held, and Flow's is the
    // same change.
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

    // Taking it selects the change, which asks for its files, and shows Changes.
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
    // The barrier is the end of the turn in the timeline, and a step after it.
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

    // Changes shown otherwise: by `f`, and the offer is not back with Flow.
    stage.keeper.tell(turn_ended(&t, Some(s2()), Some(s1())));
    wait_for_the_offer(&stage, &mut cx);
    press("f", &mut cx);
    assert_eq!(view(&stage, &cx), View::Changes);
    assert_eq!(flow_offer(&stage, &cx), None);
    press("f", &mut cx);
    assert_eq!(flow_offer(&stage, &cx), None);

    // Another turn starts: the offer of the one before is gone.
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

    // Another workspace on screen: no offer is there, nor when coming back.
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

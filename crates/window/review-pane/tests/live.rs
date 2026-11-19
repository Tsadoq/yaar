//! Component test: the live diff of Flow, on GPUI's test platform. Keys, clicks and the
//! wheel go in; the pane is read back through the model it shows. The test plays the Keeper
//! on the other end of a socket pair: it sets the agents, the turns and the settings the
//! Keeper would hold, tells the facts the Keeper tells, and answers what the live diff asks.
//!
//! A negative clause ("asks nothing") is read after a barrier: something the test causes
//! after it, through the same path, whose arrival shows that everything before it arrived.

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui_kit::{Modifiers, TestAppContext, VisualTestContext};
use yaar_diff::{Row, Side};
use yaar_link::{
    Anchor, Author, ChangedFile, Comment, CommentId, Fact, FileStatus, FlowForm, History,
    HistoryEntry, Lines, Reply, Request, Session, SessionState, Setting, Step, Thread, ThreadId,
    TurnId,
};
use yaar_review_pane::{LiveChange, PaneRow, Position, Unshown, View};

use common::{
    COMMIT, DEADLINE, Stage, agents_of, drawn, file, id, no_agent_review, open_staged, review_id,
    reviews_of, running_turn, session, session_id, since_main, step_fact, texts, turn_id,
    wait_for_files, wait_until, working_on, workspace,
};

/// The state the turn counts from.
fn s0() -> String {
    "5".repeat(40)
}

/// The state recorded at the end of the turn.
fn s1() -> String {
    "6".repeat(40)
}

/// A state the reviewer last marked reviewed, other than the turn's start.
fn s9() -> String {
    "7".repeat(40)
}

/// The start another turn is given later.
fn s5() -> String {
    "8".repeat(40)
}

fn read(path: &str) -> Step {
    Step::Read { path: path.into() }
}

fn write(path: &str) -> Step {
    Step::Write { path: path.into() }
}

/// Session `a`, which works on `turn`, as the agents model shows it.
fn session_a_on(turn: &TurnId) -> Session {
    working_on(
        session('a', "Claude Code 1", SessionState::Works, None),
        turn.clone(),
    )
}

fn session_a_idle() -> Session {
    session('a', "Claude Code 1", SessionState::Idle, None)
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

/// A click on the bar, away from every control of it: the pane has the focus afterwards.
fn focus_by_the_bar(stage: &Stage, cx: &mut VisualTestContext) {
    click("review-bar", cx);
    cx.update(|window, _| assert!(stage.focus.is_focused(window)));
}

fn live_rows(stage: &Stage, cx: &mut VisualTestContext) -> Vec<PaneRow> {
    stage
        .pane
        .read_with(cx, |pane, _| pane.live_rows().to_vec())
}

fn live_change(stage: &Stage, cx: &mut VisualTestContext) -> Option<LiveChange> {
    stage
        .pane
        .read_with(cx, |pane, _| pane.live_change().cloned())
}

/// The paths of the headings of the changed files of the live diff, in order.
fn headings(rows: &[PaneRow]) -> Vec<String> {
    rows.iter()
        .filter_map(|row| match row {
            PaneRow::File { path, .. } => Some(path.display().to_string()),
            _ => None,
        })
        .collect()
}

/// The paths of the files read, in order.
fn reads(rows: &[PaneRow]) -> Vec<String> {
    rows.iter()
        .filter_map(|row| match row {
            PaneRow::Read { path } => Some(path.display().to_string()),
            _ => None,
        })
        .collect()
}

/// Whether every file of the live diff shows its lines.
fn all_read(rows: &[PaneRow]) -> bool {
    !rows.iter().any(|row| {
        matches!(
            row,
            PaneRow::Unshown {
                why: Unshown::Loading,
                ..
            }
        )
    })
}

/// Keeps the app running until the live diff holds the changed files `wanted`, in order,
/// each with its lines read, and the files read `read` after them.
fn wait_for_live(stage: &Stage, wanted: &[&str], read: &[&str], cx: &mut VisualTestContext) {
    let deadline = Instant::now() + DEADLINE;
    loop {
        cx.run_until_parked();
        let rows = live_rows(stage, cx);
        if headings(&rows) == wanted && reads(&rows) == read && all_read(&rows) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "waited in vain for the files {wanted:?} and the reads {read:?}, the rows are {rows:#?}"
        );
        std::thread::park_timeout(Duration::from_millis(1));
    }
}

/// Keeps the app running until the files read, in the live diff, are `wanted`: a fact the
/// Keeper told reaches the pane by a thread of its own, which the app does not wait for.
fn wait_for_reads(stage: &Stage, wanted: &[&str], cx: &mut VisualTestContext) {
    let deadline = Instant::now() + DEADLINE;
    loop {
        cx.run_until_parked();
        let held = reads(&live_rows(stage, cx));
        if held == wanted {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "waited in vain for the files read {wanted:?}, which are {held:?}"
        );
        std::thread::park_timeout(Duration::from_millis(1));
    }
}

/// Answers the next request, which has to list the files of the change from `from` to
/// `to`, with `files`.
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

/// Answers the next requests, as many as `files` has rows, each of which has to read the
/// two versions of one of them, from `old` to `new`, in any order: a few are on their way
/// at once.
fn answer_reads(
    stage: &mut Stage,
    old: &str,
    new: Option<&str>,
    files: &[(&str, &str, &str)],
    cx: &mut VisualTestContext,
) {
    let mut left: Vec<&(&str, &str, &str)> = files.iter().collect();
    while !left.is_empty() {
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
        assert_eq!(old_path, path, "no file of these tests was renamed");
        let at = left
            .iter()
            .position(|(name, ..)| PathBuf::from(name) == path)
            .unwrap_or_else(|| panic!("{path:?} was read, which is not among {left:?}"));
        let (_, before, after) = left.remove(at);
        stage
            .keeper
            .reply(asked, texts(before, after).map(Reply::File));
    }
}

/// A pane on workspace `1` in the live form of Flow, with session `a` idle and no turn,
/// the changes of the workspace answered, and the focus in the pane, Flow on screen.
fn in_the_live_diff(cx: &mut TestAppContext) -> (Stage, VisualTestContext) {
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature")],
        |keeper| {
            keeper.set_settings(vec![Setting::FlowForm {
                workspace: id('1'),
                form: FlowForm::LiveDiff,
            }]);
            keeper.set_agents(vec![agents_of(vec![session_a_idle()])]);
        },
        cx,
    );
    let first = since_main(Vec::new(), vec![file("z.rs", FileStatus::Modified)]);
    stage.keeper.show(&first, |_| texts("a\n", "b\n"), &mut cx);
    wait_for_files(&stage.pane, &mut cx);
    focus_by_the_bar(&stage, &mut cx);
    press("f", &mut cx);
    assert_eq!(stage.pane.read_with(&cx, |pane, _| pane.view()), View::Flow);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.flow_form()),
        FlowForm::LiveDiff
    );
    (stage, cx)
}

/// Starts turn `turn` of session `a`, counted from `start`, in the workspace on screen.
fn start_turn(stage: &mut Stage, turn: &TurnId, start: &str, cx: &mut VisualTestContext) {
    stage.keeper.set_turn(running_turn(
        turn.clone(),
        session_id('a'),
        1,
        Some(start),
        Vec::new(),
    ));
    stage
        .keeper
        .tell_agents(agents_of(vec![session_a_on(turn)]));
    cx.run_until_parked();
}

/// What the Frame asks when the files of the workspace change, which the pane answers with
/// no file of Changes: answered, in either order with what the live diff asks of its own,
/// `files` being what it is told of the turn's change from `from` to the files on disk.
fn the_files_changed(
    stage: &mut Stage,
    from: &str,
    files: Vec<ChangedFile>,
    cx: &mut VisualTestContext,
) {
    stage
        .keeper
        .tell(Fact::ChangesChanged { workspace: id('1') });
    let mut compared_yet = false;
    let mut told_yet = false;
    while !(compared_yet && told_yet) {
        let (asked, request) = stage.keeper.request(cx);
        match request {
            Request::GetChanges { workspace } => {
                assert_eq!(workspace, id('1'));
                told_yet = true;
                let changes = since_main(Vec::new(), Vec::new());
                stage.keeper.reply(asked, Ok(Reply::Changes(changes)));
            }
            Request::CompareVersions {
                workspace,
                from: counted_from,
                to,
            } => {
                assert_eq!(
                    (workspace, counted_from.as_str(), to),
                    (id('1'), from, None)
                );
                compared_yet = true;
                stage
                    .keeper
                    .reply(asked, Ok(Reply::Compared(files.clone())));
            }
            other => panic!("the files changed, and the Window asked {other:?}"),
        }
    }
}

#[gpui_kit::test]
fn the_live_diff_shows_the_diff_of_the_running_turn_and_grows_as_writes_arrive(
    cx: &mut TestAppContext,
) {
    let t = turn_id('t');
    let (mut stage, mut cx) = in_the_live_diff(cx);

    // With no turn there is nothing to show, and nothing is asked: the barrier is the
    // turn that starts next, whose list is the next request.
    assert!(drawn("flow-live", &mut cx).is_some());
    assert!(drawn("flow-empty", &mut cx).is_some());
    assert_eq!(live_change(&stage, &mut cx), None);
    assert_eq!(live_rows(&stage, &mut cx), []);
    start_turn(&mut stage, &t, &s0(), &mut cx);
    compared(
        &mut stage,
        &s0(),
        None,
        vec![file("a.txt", FileStatus::Modified)],
        &mut cx,
    );

    // The diff of the running turn: the files it changed so far against the state it
    // started from.
    answer_reads(
        &mut stage,
        &s0(),
        None,
        &[("a.txt", "one\n", "one\ntwo\n")],
        &mut cx,
    );
    wait_for_live(&stage, &["a.txt"], &[], &mut cx);
    assert_eq!(
        live_change(&stage, &mut cx),
        Some(LiveChange {
            workspace: id('1'),
            turn: t.clone(),
            from: Some(s0()),
            to: None,
        })
    );
    let rows = live_rows(&stage, &mut cx);
    let a = PathBuf::from("a.txt");
    assert_eq!(
        rows,
        [
            PaneRow::File {
                path: a.clone(),
                status: FileStatus::Modified,
            },
            PaneRow::Diff {
                path: a.clone(),
                row: Row::Context { old: 0, new: 0 },
            },
            PaneRow::Diff {
                path: a.clone(),
                row: Row::Added { new: 1 },
            },
        ]
    );
    let two = Position {
        path: a.clone(),
        side: Side::New,
        line: 1,
    };
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.live_text(&two)),
        Some("two".to_owned())
    );
    assert!(drawn("flow-row-0", &mut cx).is_some());
    assert!(drawn("flow-text-2-new:two", &mut cx).is_some());
    assert!(drawn("flow-empty", &mut cx).is_none());

    // It grows as writes arrive: a file the agent wrote whole, then a file a shell
    // command wrote, which only the watcher tells of.
    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 1, write("b.txt")));
    compared(
        &mut stage,
        &s0(),
        None,
        vec![
            file("a.txt", FileStatus::Modified),
            file("b.txt", FileStatus::Added),
        ],
        &mut cx,
    );
    // Only the file new to the list is read: a.txt is not.
    answer_reads(
        &mut stage,
        &s0(),
        None,
        &[("b.txt", "", "new b\n")],
        &mut cx,
    );
    wait_for_live(&stage, &["a.txt", "b.txt"], &[], &mut cx);
    the_files_changed(
        &mut stage,
        &s0(),
        vec![
            file("a.txt", FileStatus::Modified),
            file("b.txt", FileStatus::Added),
            file("c.txt", FileStatus::Added),
        ],
        &mut cx,
    );
    // After a change the watcher told of, every file is read again, in any order.
    answer_reads(
        &mut stage,
        &s0(),
        None,
        &[
            ("a.txt", "one\n", "one\ntwo\n"),
            ("b.txt", "", "new b\n"),
            ("c.txt", "", "new c\n"),
        ],
        &mut cx,
    );
    wait_for_live(&stage, &["a.txt", "b.txt", "c.txt"], &[], &mut cx);
    let changed = live_rows(&stage, &mut cx).len();

    // The files it read and did not change come after every file it changed, one row
    // each, with no request: the row is there before anything is answered.
    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 2, read("r.txt")));
    wait_for_reads(&stage, &["r.txt"], &mut cx);
    let rows = live_rows(&stage, &mut cx);
    assert_eq!(rows.len(), changed + 1);
    assert_eq!(
        rows.last(),
        Some(&PaneRow::Read {
            path: "r.txt".into()
        })
    );
    assert_eq!(headings(&rows), ["a.txt", "b.txt", "c.txt"]);
    assert!(
        !rows.iter().any(|row| matches!(
            row,
            PaneRow::File { path, .. } | PaneRow::Diff { path, .. } | PaneRow::Unshown { path, .. }
                if path == &PathBuf::from("r.txt")
        )),
        "a file read has no heading, hunk or lines: {rows:#?}"
    );
    assert!(drawn(&format!("flow-read-{}", rows.len() - 1), &mut cx).is_some());
    // A second file read comes after the first; a file read again is not listed twice.
    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 3, read("q.txt")));
    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 4, read("r.txt")));
    wait_for_reads(&stage, &["r.txt", "q.txt"], &mut cx);
    let rows = live_rows(&stage, &mut cx);
    assert_eq!(rows.len(), changed + 2);
    assert_eq!(reads(&rows), ["r.txt", "q.txt"]);
    assert!(matches!(rows[rows.len() - 2], PaneRow::Read { .. }));

    // It leaves the group when the turn changes it: while the list that write asked for
    // is not answered, r.txt is still a file read.
    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 5, write("r.txt")));
    let (asked, request) = stage.keeper.request(&mut cx);
    assert_eq!(
        request,
        Request::CompareVersions {
            workspace: id('1'),
            from: s0(),
            to: None,
        },
        "the reads asked nothing: the first request is the write's"
    );
    cx.run_until_parked();
    assert_eq!(reads(&live_rows(&stage, &mut cx)), ["r.txt", "q.txt"]);
    stage.keeper.reply(
        asked,
        Ok(Reply::Compared(vec![
            file("a.txt", FileStatus::Modified),
            file("b.txt", FileStatus::Added),
            file("c.txt", FileStatus::Added),
            file("r.txt", FileStatus::Modified),
        ])),
    );
    answer_reads(
        &mut stage,
        &s0(),
        None,
        &[("r.txt", "old r\n", "new r\n")],
        &mut cx,
    );
    wait_for_live(
        &stage,
        &["a.txt", "b.txt", "c.txt", "r.txt"],
        &["q.txt"],
        &mut cx,
    );
    let rows = live_rows(&stage, &mut cx);
    assert_eq!(
        rows.last(),
        Some(&PaneRow::Read {
            path: "q.txt".into()
        })
    );

    // Hidden, it asks nothing, and asks once when it comes on screen again. The barrier
    // for the first is a change the Frame asks the Keeper about, told twice: no question
    // of the live diff comes between the Frame's.
    press("f", &mut cx);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.view()),
        View::Changes
    );
    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 6, write("d.txt")));
    for _ in 0..2 {
        stage
            .keeper
            .tell(Fact::ChangesChanged { workspace: id('1') });
        let (asked, request) = stage.keeper.request(&mut cx);
        assert_eq!(request, Request::GetChanges { workspace: id('1') });
        stage.keeper.reply(
            asked,
            Ok(Reply::Changes(since_main(Vec::new(), Vec::new()))),
        );
    }
    press("f", &mut cx);
    assert_eq!(stage.pane.read_with(&cx, |pane, _| pane.view()), View::Flow);
    let all = vec![
        file("a.txt", FileStatus::Modified),
        file("b.txt", FileStatus::Added),
        file("c.txt", FileStatus::Added),
        file("r.txt", FileStatus::Modified),
        file("d.txt", FileStatus::Added),
    ];
    compared(&mut stage, &s0(), None, all, &mut cx);
    let every_file = [
        ("a.txt", "one\n", "one\ntwo\n"),
        ("b.txt", "", "new b\n"),
        ("c.txt", "", "new c\n"),
        ("r.txt", "old r\n", "new r\n"),
        ("d.txt", "", "new d\n"),
    ];
    answer_reads(&mut stage, &s0(), None, &every_file, &mut cx);
    wait_for_live(
        &stage,
        &["a.txt", "b.txt", "c.txt", "r.txt", "d.txt"],
        &["q.txt"],
        &mut cx,
    );

    // The turn ends with a state recorded, under the default setting, which selects what
    // changed since the last review in Changes: that list is asked first, and the live
    // diff's after it, now to the state the turn ended with.
    stage.keeper.tell(Fact::TurnEnded {
        workspace: id('1'),
        session: session_id('a'),
        turn: t.clone(),
        state: Some(s1()),
        since: Some(s9()),
    });
    compared(&mut stage, &s9(), Some(&s1()), Vec::new(), &mut cx);
    compared(
        &mut stage,
        &s0(),
        Some(&s1()),
        vec![
            file("a.txt", FileStatus::Modified),
            file("b.txt", FileStatus::Added),
            file("c.txt", FileStatus::Added),
            file("r.txt", FileStatus::Modified),
            file("d.txt", FileStatus::Added),
        ],
        &mut cx,
    );
    answer_reads(&mut stage, &s0(), Some(&s1()), &every_file, &mut cx);
    wait_for_live(
        &stage,
        &["a.txt", "b.txt", "c.txt", "r.txt", "d.txt"],
        &["q.txt"],
        &mut cx,
    );
    assert_eq!(
        live_change(&stage, &mut cx),
        Some(LiveChange {
            workspace: id('1'),
            turn: t.clone(),
            from: Some(s0()),
            to: Some(s1()),
        })
    );
    assert_eq!(stage.pane.read_with(&cx, |pane, _| pane.view()), View::Flow);

    // The running turn is the one shown while its start is not known: it counts from the
    // commit checked out, and starts over from its start once that is kept.
    let t2 = turn_id('u');
    let b = session('b', "Claude Code 2", SessionState::Idle, None);
    stage.keeper.set_turn(running_turn(
        t2.clone(),
        session_id('b'),
        1,
        None,
        Vec::new(),
    ));
    stage
        .keeper
        .tell_agents(agents_of(vec![session_a_idle(), working_on(b, t2.clone())]));
    compared(&mut stage, COMMIT, None, Vec::new(), &mut cx);
    cx.run_until_parked();
    assert_eq!(
        live_change(&stage, &mut cx),
        Some(LiveChange {
            workspace: id('1'),
            turn: t2.clone(),
            from: Some(COMMIT.to_owned()),
            to: None,
        })
    );
    assert_eq!(live_rows(&stage, &mut cx), []);
    // The start is kept when the turn before is recorded: the Window reads the turn again
    // at the end of another turn of the workspace, and counts from it.
    stage.keeper.set_turn(running_turn(
        t2.clone(),
        session_id('b'),
        1,
        Some(&s5()),
        Vec::new(),
    ));
    press("shift-t", &mut cx);
    let before = stage
        .keeper
        .turns_asked()
        .iter()
        .filter(|asked| **asked == t2)
        .count();
    stage.keeper.tell(Fact::TurnEnded {
        workspace: id('1'),
        session: session_id('b'),
        turn: turn_id('v'),
        state: Some(s5()),
        since: None,
    });
    compared(&mut stage, &s5(), None, Vec::new(), &mut cx);
    assert_eq!(
        stage
            .keeper
            .turns_asked()
            .iter()
            .filter(|asked| **asked == t2)
            .count(),
        before + 1
    );
    assert_eq!(
        live_change(&stage, &mut cx).map(|change| (change.turn, change.from)),
        Some((t2, Some(s5())))
    );
}

#[gpui_kit::test]
fn a_turn_that_ended_before_the_window_opened_is_shown_to_the_state_it_ended_with(
    cx: &mut TestAppContext,
) {
    let t = turn_id('t');
    let ended = yaar_link::Turn {
        ended: Some(yaar_link::TurnEnd::Finished),
        state: Some(s1()),
        steps: vec![read("notes.txt"), write("a.txt")],
        ..running_turn(t.clone(), session_id('a'), 1, Some(&s0()), Vec::new())
    };
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature")],
        |keeper| {
            keeper.set_settings(vec![Setting::FlowForm {
                workspace: id('1'),
                form: FlowForm::LiveDiff,
            }]);
            keeper.set_agents(vec![agents_of(vec![session_a_idle()])]);
            keeper.set_history(History {
                workspace: id('1'),
                start: Some(COMMIT.to_owned()),
                entries: vec![HistoryEntry::Turn {
                    turn: t.clone(),
                    session: session_id('a'),
                    state: Some(s1()),
                }],
            });
            keeper.set_turn(ended);
        },
        cx,
    );
    let changes = since_main(Vec::new(), Vec::new());
    stage.keeper.show(&changes, |_| texts("", ""), &mut cx);
    focus_by_the_bar(&stage, &mut cx);
    press("f", &mut cx);
    // A window with no changes of its own shows the last turn recorded in Changes, which
    // asks too, and in no fixed order with the live diff: the live diff's questions are
    // told by what they count from.
    let mut read_a = false;
    while !read_a {
        let (asked, request) = stage.keeper.request(&mut cx);
        match request {
            Request::CompareVersions { from, to, .. } if from == COMMIT => {
                assert_eq!(to, Some(s1()));
                stage.keeper.reply(asked, Ok(Reply::Compared(Vec::new())));
            }
            Request::CompareVersions { from, to, .. } => {
                assert_eq!((from, to), (s0(), Some(s1())));
                let files = vec![file("a.txt", FileStatus::Added)];
                stage.keeper.reply(asked, Ok(Reply::Compared(files)));
            }
            Request::ReadVersions { old, new, path, .. } => {
                assert_eq!(
                    (old, new, path),
                    (Some(s0()), Some(s1()), PathBuf::from("a.txt"))
                );
                stage.keeper.reply(asked, texts("", "x\n").map(Reply::File));
                read_a = true;
            }
            other => panic!("the Window asked {other:?}"),
        }
    }
    wait_for_live(&stage, &["a.txt"], &["notes.txt"], &mut cx);
    assert_eq!(
        live_change(&stage, &mut cx),
        Some(LiveChange {
            workspace: id('1'),
            turn: t,
            from: Some(s0()),
            to: Some(s1()),
        })
    );
}

#[gpui_kit::test]
fn a_file_read_outside_the_worktree_is_listed_by_its_whole_path_and_a_file_read_under_the_name_a_renamed_file_had_is_no_file_read(
    cx: &mut TestAppContext,
) {
    let t = turn_id('t');
    let (mut stage, mut cx) = in_the_live_diff(cx);
    start_turn(&mut stage, &t, &s0(), &mut cx);
    compared(
        &mut stage,
        &s0(),
        None,
        vec![ChangedFile {
            path: "new name.txt".into(),
            status: FileStatus::Renamed {
                from: "old name.txt".into(),
            },
        }],
        &mut cx,
    );
    let (asked, request) = stage.keeper.request(&mut cx);
    assert_eq!(
        request,
        Request::ReadVersions {
            workspace: id('1'),
            old: Some(s0()),
            old_path: "old name.txt".into(),
            new: None,
            path: "new name.txt".into(),
        },
        "a renamed file is read by the path it had before"
    );
    stage
        .keeper
        .reply(asked, texts("x\n", "x\n").map(Reply::File));
    wait_for_live(&stage, &["new name.txt"], &[], &mut cx);
    // Before the list is here nothing is taken out of the group; once it is, the old
    // name of a renamed file is the file that changed.
    stage.keeper.tell(step_fact(
        session_id('a'),
        t.clone(),
        1,
        read("old name.txt"),
    ));
    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 2, read("/etc/hosts")));
    // The first read came before the second, which is here: the old name was left out.
    wait_for_reads(&stage, &["/etc/hosts"], &mut cx);
}

#[gpui_kit::test]
fn j_and_k_move_through_the_lines_of_the_live_diff_across_its_files_and_pass_the_files_read_by(
    cx: &mut TestAppContext,
) {
    let t = turn_id('t');
    let (mut stage, mut cx) = in_the_live_diff(cx);
    start_turn(&mut stage, &t, &s0(), &mut cx);
    compared(
        &mut stage,
        &s0(),
        None,
        vec![
            file("a.txt", FileStatus::Modified),
            file("b.txt", FileStatus::Modified),
        ],
        &mut cx,
    );
    answer_reads(
        &mut stage,
        &s0(),
        None,
        &[("a.txt", "one\n", "uno\n"), ("b.txt", "x\ny\n", "x\n")],
        &mut cx,
    );
    wait_for_live(&stage, &["a.txt", "b.txt"], &[], &mut cx);
    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 1, read("r.txt")));
    cx.run_until_parked();
    let at = |stage: &Stage, cx: &mut VisualTestContext| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.live_position().cloned())
            .map(|position| {
                (
                    position.path.display().to_string(),
                    position.side,
                    position.line,
                )
            })
    };
    // The reviewer starts on the first changed line.
    assert_eq!(
        at(&stage, &mut cx),
        Some(("a.txt".to_owned(), Side::Old, 0))
    );
    press("j", &mut cx);
    assert_eq!(
        at(&stage, &mut cx),
        Some(("a.txt".to_owned(), Side::New, 0))
    );
    press("j", &mut cx);
    assert_eq!(
        at(&stage, &mut cx),
        Some(("b.txt".to_owned(), Side::New, 0))
    );
    press("j", &mut cx);
    assert_eq!(
        at(&stage, &mut cx),
        Some(("b.txt".to_owned(), Side::Old, 1))
    );
    // The end: the file read has no line to stand on, so `j` stays on the last line.
    press("j", &mut cx);
    assert_eq!(
        at(&stage, &mut cx),
        Some(("b.txt".to_owned(), Side::Old, 1))
    );
    press("k", &mut cx);
    assert_eq!(
        at(&stage, &mut cx),
        Some(("b.txt".to_owned(), Side::New, 0))
    );
    // A click on a line puts the reviewer there.
    let rows = live_rows(&stage, &mut cx);
    let index = rows
        .iter()
        .position(|row| {
            matches!(row, PaneRow::Diff { path, row: Row::Added { .. } } if path == &PathBuf::from("a.txt"))
        })
        .expect("a.txt has an added line");
    click(&format!("flow-text-{index}-new:uno"), &mut cx);
    assert_eq!(
        at(&stage, &mut cx),
        Some(("a.txt".to_owned(), Side::New, 0))
    );
}

/// A live diff of turn `t` that holds `a.txt`, `one` and then `two` added.
fn with_a_change(t: &TurnId, cx: &mut TestAppContext) -> (Stage, VisualTestContext) {
    let (mut stage, mut cx) = in_the_live_diff(cx);
    start_turn(&mut stage, t, &s0(), &mut cx);
    compared(
        &mut stage,
        &s0(),
        None,
        vec![file("a.txt", FileStatus::Modified)],
        &mut cx,
    );
    answer_reads(
        &mut stage,
        &s0(),
        None,
        &[("a.txt", "one\n", "one\ntwo\n")],
        &mut cx,
    );
    wait_for_live(&stage, &["a.txt"], &[], &mut cx);
    (stage, cx)
}

#[gpui_kit::test]
fn the_threads_of_the_review_sit_under_the_lines_of_the_live_diff_they_are_on(
    cx: &mut TestAppContext,
) {
    let t = turn_id('t');
    let (mut stage, mut cx) = with_a_change(&t, cx);
    let thread = Thread {
        id: ThreadId::new("7".repeat(32)),
        anchor: Anchor {
            tree: None,
            path: "a.txt".into(),
            side: yaar_link::Side::New,
            // Lines are counted from 1 here: this is the line `two`.
            lines: Lines { first: 2, last: 2 },
            text: "two".to_owned(),
        },
        resolved: false,
        addressed: false,
        comments: vec![Comment {
            id: CommentId::new("8".repeat(32)),
            author: Author::Reviewer,
            text: "why?".to_owned(),
            round: None,
        }],
    };
    let mut review = no_agent_review();
    review.threads.push(thread.clone());
    assert_eq!(review.id, review_id());
    stage.keeper.set_reviews(reviews_of(vec![review]));
    stage
        .keeper
        .tell(Fact::ReviewsChanged { workspace: id('1') });
    wait_until(&mut cx, "the thread under its line", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.live_rows()
                .iter()
                .any(|row| matches!(row, PaneRow::Thread { .. }))
        })
    });
    let rows = live_rows(&stage, &mut cx);
    let at = rows.len() - 2;
    assert_eq!(
        rows[at - 1],
        PaneRow::Diff {
            path: "a.txt".into(),
            row: Row::Added { new: 1 },
        }
    );
    assert_eq!(
        rows[at],
        PaneRow::Thread {
            thread: thread.id.clone()
        }
    );
    assert_eq!(
        rows[at + 1],
        PaneRow::Comment {
            thread: thread.id.clone(),
            comment: thread.comments[0].id.clone(),
        }
    );
    assert!(drawn(&format!("flow-thread-{}", thread.id), &mut cx).is_some());
    assert!(
        drawn(
            &format!("flow-comment-{}:draft", thread.comments[0].id),
            &mut cx
        )
        .is_some()
    );
}

#[gpui_kit::test]
fn the_layout_switch_makes_the_rows_of_the_live_diff_again_in_the_other_layout(
    cx: &mut TestAppContext,
) {
    let t = turn_id('t');
    let (stage, mut cx) = with_a_change(&t, cx);
    let unified = live_rows(&stage, &mut cx);
    assert!(unified.iter().any(|row| matches!(
        row,
        PaneRow::Diff {
            row: Row::Added { .. },
            ..
        }
    )));
    press("s", &mut cx);
    let split = live_rows(&stage, &mut cx);
    assert_ne!(split, unified);
    assert!(
        split.iter().any(|row| matches!(
            row,
            PaneRow::Diff {
                row: Row::Changed {
                    old: None,
                    new: Some(1)
                },
                ..
            }
        )),
        "{split:#?}"
    );
    assert!(drawn("flow-row-2-new", &mut cx).is_some());
}

//! A negative clause is read after a barrier: a later request through the same path, whose arrival
//! shows the earlier ones arrived.

mod common;

use std::path::PathBuf;

use gpui_kit::{Modifiers, TestAppContext, VisualTestContext};
use yaar_diff::{Row, Side};
use yaar_link::{
    Anchor, Author, ChangedFile, Comment, CommentId, Fact, FileStatus, FlowForm, History,
    HistoryEntry, Lines, Refusal, Reply, Request, Session, SessionState, Setting, Step, Thread,
    ThreadId, TurnId,
};
use yaar_review_pane::{LiveChange, PaneRow, Position, Unshown, View};

use common::{
    COMMIT, Stage, agents_of, drawn, file, id, no_agent_review, open_staged, review_id, reviews_of,
    running_turn, session, session_id, since_main, step_fact, texts, turn_id, wait_for_files,
    wait_until, working_on, workspace,
};

fn s0() -> String {
    "5".repeat(40)
}

fn s1() -> String {
    "6".repeat(40)
}

fn s9() -> String {
    "7".repeat(40)
}

fn s5() -> String {
    "8".repeat(40)
}

fn read(path: &str) -> Step {
    Step::Read {
        path: path.into(),
        lines: None,
    }
}

fn read_lines(path: &str, first: u32, last: u32) -> Step {
    Step::Read {
        path: path.into(),
        lines: Some(first..=last),
    }
}

fn write(path: &str) -> Step {
    Step::Write { path: path.into() }
}

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

fn headings(rows: &[PaneRow]) -> Vec<String> {
    rows.iter()
        .filter_map(|row| match row {
            PaneRow::File { path, .. } => Some(path.display().to_string()),
            _ => None,
        })
        .collect()
}

fn reads(rows: &[PaneRow]) -> Vec<String> {
    rows.iter()
        .filter_map(|row| match row {
            PaneRow::Read { path } => Some(path.display().to_string()),
            _ => None,
        })
        .collect()
}

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

fn wait_for_live(stage: &Stage, wanted: &[&str], read: &[&str], cx: &mut VisualTestContext) {
    let what = format!("the files {wanted:?} and the reads {read:?}");
    wait_until(cx, &what, |cx| {
        let rows = stage
            .pane
            .read_with(cx, |pane, _| pane.live_rows().to_vec());
        headings(&rows) == wanted && reads(&rows) == read && all_read(&rows)
    });
}

fn wait_for_reads(stage: &Stage, wanted: &[&str], cx: &mut VisualTestContext) {
    let what = format!("the files read {wanted:?}");
    wait_until(cx, &what, |cx| {
        reads(
            &stage
                .pane
                .read_with(cx, |pane, _| pane.live_rows().to_vec()),
        ) == wanted
    });
}

fn wait_for_row(stage: &Stage, row: &PaneRow, cx: &mut VisualTestContext) {
    wait_until(cx, &format!("the row {row:?}"), |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.live_rows().contains(row))
    });
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

fn a_read_and_the_next_list(
    stage: &mut Stage,
    from: &str,
    read: (&str, &str, &str),
    files: Vec<ChangedFile>,
    cx: &mut VisualTestContext,
) {
    let mut list = None;
    let mut read_yet = false;
    while list.is_none() || !read_yet {
        let (asked, request) = stage.keeper.request(cx);
        match request {
            Request::ReadVersions {
                workspace,
                old,
                old_path,
                new,
                path,
            } => {
                assert!(!read_yet, "{path:?} was read twice");
                assert_eq!(
                    (workspace, old.as_deref(), new, old_path, path),
                    (id('1'), Some(from), None, read.0.into(), read.0.into())
                );
                read_yet = true;
                stage
                    .keeper
                    .reply(asked, texts(read.1, read.2).map(Reply::File));
            }
            Request::CompareVersions {
                workspace,
                from: counted_from,
                to,
            } => {
                assert!(list.is_none(), "the list was asked twice");
                assert_eq!(
                    (workspace, counted_from.as_str(), to),
                    (id('1'), from, None)
                );
                list = Some(asked);
            }
            other => panic!("a read and a list were expected, not {other:?}"),
        }
    }
    if let Some(asked) = list {
        stage.keeper.reply(asked, Ok(Reply::Compared(files)));
    }
}

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

    // With no turn there is nothing to show and nothing is asked: the barrier is the turn that
    // starts next, whose list is the next request.
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

    // A file the agent edits again is read again, though the list is as before. Only that file is
    // read, which is also the barrier for the three reads above: a read that had asked for the list
    // would have its question come first.
    stage.keeper.tell(step_fact(
        session_id('a'),
        t.clone(),
        5,
        Step::Edit {
            path: "a.txt".into(),
            lines: None,
        },
    ));
    compared(
        &mut stage,
        &s0(),
        None,
        vec![
            file("a.txt", FileStatus::Modified),
            file("b.txt", FileStatus::Added),
            file("c.txt", FileStatus::Added),
        ],
        &mut cx,
    );
    answer_reads(
        &mut stage,
        &s0(),
        None,
        &[("a.txt", "one\n", "one\ntwo\nthree\n")],
        &mut cx,
    );
    wait_for_row(
        &stage,
        &PaneRow::Diff {
            path: a.clone(),
            row: Row::Added { new: 2 },
        },
        &mut cx,
    );
    wait_for_live(
        &stage,
        &["a.txt", "b.txt", "c.txt"],
        &["r.txt", "q.txt"],
        &mut cx,
    );

    let a_is = |status: FileStatus| PaneRow::File {
        path: a.clone(),
        status,
    };
    the_files_changed(
        &mut stage,
        &s0(),
        vec![
            file("a.txt", FileStatus::Deleted),
            file("b.txt", FileStatus::Added),
            file("c.txt", FileStatus::Added),
        ],
        &mut cx,
    );
    answer_reads(
        &mut stage,
        &s0(),
        None,
        &[
            ("a.txt", "one\ntwo\nthree\n", ""),
            ("b.txt", "", "new b\n"),
            ("c.txt", "", "new c\n"),
        ],
        &mut cx,
    );
    wait_for_row(&stage, &a_is(FileStatus::Deleted), &mut cx);
    wait_for_live(
        &stage,
        &["a.txt", "b.txt", "c.txt"],
        &["r.txt", "q.txt"],
        &mut cx,
    );
    let rows = live_rows(&stage, &mut cx);
    assert!(!rows.contains(&a_is(FileStatus::Modified)), "{rows:#?}");
    let the_three = vec![
        file("a.txt", FileStatus::Modified),
        file("b.txt", FileStatus::Added),
        file("c.txt", FileStatus::Added),
    ];
    the_files_changed(&mut stage, &s0(), the_three, &mut cx);
    answer_reads(
        &mut stage,
        &s0(),
        None,
        &[
            ("a.txt", "one\n", "one\ntwo\nthree\n"),
            ("b.txt", "", "new b\n"),
            ("c.txt", "", "new c\n"),
        ],
        &mut cx,
    );
    wait_for_row(&stage, &a_is(FileStatus::Modified), &mut cx);
    wait_for_live(
        &stage,
        &["a.txt", "b.txt", "c.txt"],
        &["r.txt", "q.txt"],
        &mut cx,
    );
    let rows = live_rows(&stage, &mut cx);
    assert!(!rows.contains(&a_is(FileStatus::Deleted)), "{rows:#?}");

    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 6, write("r.txt")));
    let (asked, request) = stage.keeper.request(&mut cx);
    assert_eq!(
        request,
        Request::CompareVersions {
            workspace: id('1'),
            from: s0(),
            to: None,
        },
        "the first request is the write's list"
    );
    cx.run_until_parked();
    assert_eq!(reads(&live_rows(&stage, &mut cx)), ["r.txt", "q.txt"]);
    // One question for the list is on its way at a time: a write and a watcher change, meanwhile,
    // ask nothing, so the Frame's question about the changes is the next request.
    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 7, write("a.txt")));
    stage
        .keeper
        .tell(Fact::ChangesChanged { workspace: id('1') });
    let (frame, request) = stage.keeper.request(&mut cx);
    assert_eq!(request, Request::GetChanges { workspace: id('1') });
    stage.keeper.reply(
        frame,
        Ok(Reply::Changes(since_main(Vec::new(), Vec::new()))),
    );
    let four = || {
        vec![
            file("a.txt", FileStatus::Modified),
            file("b.txt", FileStatus::Added),
            file("c.txt", FileStatus::Added),
            file("r.txt", FileStatus::Modified),
        ]
    };
    stage.keeper.reply(asked, Ok(Reply::Compared(four())));
    a_read_and_the_next_list(
        &mut stage,
        &s0(),
        ("r.txt", "old r\n", "new r\n"),
        four(),
        &mut cx,
    );
    answer_reads(
        &mut stage,
        &s0(),
        None,
        &[
            ("a.txt", "one\n", "one\ntwo\nthree\n"),
            ("b.txt", "", "new b\n"),
            ("c.txt", "", "new c\n"),
            ("r.txt", "old r\n", "new r\n"),
        ],
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

    press("shift-l", &mut cx);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.flow_form()),
        FlowForm::Timeline
    );
    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 8, write("a.txt")));
    stage
        .keeper
        .tell(Fact::ChangesChanged { workspace: id('1') });
    let (frame, request) = stage.keeper.request(&mut cx);
    assert_eq!(request, Request::GetChanges { workspace: id('1') });
    stage.keeper.reply(
        frame,
        Ok(Reply::Changes(since_main(Vec::new(), Vec::new()))),
    );
    press("shift-l", &mut cx);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.flow_form()),
        FlowForm::LiveDiff
    );
    compared(&mut stage, &s0(), None, four(), &mut cx);
    answer_reads(
        &mut stage,
        &s0(),
        None,
        &[
            ("a.txt", "one\n", "one\ntwo\nthree\n"),
            ("b.txt", "", "new b\n"),
            ("c.txt", "", "new c\n"),
            ("r.txt", "old r\n", "new r\n"),
        ],
        &mut cx,
    );
    wait_for_live(
        &stage,
        &["a.txt", "b.txt", "c.txt", "r.txt"],
        &["q.txt"],
        &mut cx,
    );

    // Hidden, the live diff asks nothing, not even for a list on its way, and asks once on return.
    // The barrier is a change the Frame asks the Keeper about, told twice.
    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 9, write("d.txt")));
    let (asked, request) = stage.keeper.request(&mut cx);
    assert_eq!(
        request,
        Request::CompareVersions {
            workspace: id('1'),
            from: s0(),
            to: None,
        }
    );
    press("f", &mut cx);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.view()),
        View::Changes
    );
    let mut all = four();
    all.push(file("d.txt", FileStatus::Added));
    stage.keeper.reply(asked, Ok(Reply::Compared(all.clone())));
    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 10, write("a.txt")));
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
    a_read_and_the_next_list(&mut stage, &s0(), ("d.txt", "", "new d\n"), all, &mut cx);
    let every_file = [
        ("a.txt", "one\n", "one\ntwo\nthree\n"),
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

    // The turn's change runs between two recorded states that no later write changes, so a watcher
    // change asks nothing of the live diff. The barrier is the Frame's own question, told twice:
    // a question of the live diff would come between the two.
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

    // While its start is not known the running turn counts from the commit checked out,
    // and from its start once that is kept.
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
    stage.keeper.tell(step_fact(
        session_id('b'),
        t2.clone(),
        1,
        read("seen by u.txt"),
    ));
    wait_for_reads(&stage, &["seen by u.txt"], &mut cx);
    assert_eq!(
        live_rows(&stage, &mut cx),
        [PaneRow::Read {
            path: "seen by u.txt".into()
        }]
    );
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
        2,
        "the turn was asked for once more, as {before} before"
    );
    assert_eq!(
        live_change(&stage, &mut cx).map(|change| (change.turn, change.from)),
        Some((t2, Some(s5())))
    );
}

#[gpui_kit::test]
fn a_list_the_keeper_refused_is_asked_again_with_the_files_the_writes_named_before(
    cx: &mut TestAppContext,
) {
    let t = turn_id('t');
    let (mut stage, mut cx) = in_the_live_diff(cx);
    start_turn(&mut stage, &t, &s0(), &mut cx);
    let three = || {
        vec![
            file("a.txt", FileStatus::Modified),
            file("b.txt", FileStatus::Added),
            file("c.txt", FileStatus::Added),
        ]
    };
    compared(&mut stage, &s0(), None, three(), &mut cx);
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

    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 1, write("a.txt")));
    let asked = stage.keeper.expect(
        Request::CompareVersions {
            workspace: id('1'),
            from: s0(),
            to: None,
        },
        &mut cx,
    );
    stage.keeper.reply(
        asked,
        Err(Refusal::Failed {
            reason: "the worktree is busy".to_owned(),
        }),
    );
    wait_for_live(&stage, &["a.txt", "b.txt", "c.txt"], &[], &mut cx);

    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 2, write("c.txt")));
    compared(&mut stage, &s0(), None, three(), &mut cx);
    answer_reads(
        &mut stage,
        &s0(),
        None,
        &[
            ("a.txt", "one\n", "one\ntwo\nthree\n"),
            ("c.txt", "", "newer c\n"),
        ],
        &mut cx,
    );
    wait_for_row(
        &stage,
        &PaneRow::Diff {
            path: "a.txt".into(),
            row: Row::Added { new: 2 },
        },
        &mut cx,
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
    // A window with no changes of its own shows the last turn recorded in Changes, which asks too,
    // in no fixed order with the live diff.
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
    stage.keeper.tell(step_fact(
        session_id('a'),
        t.clone(),
        1,
        read("old name.txt"),
    ));
    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 2, read("/etc/hosts")));
    wait_for_reads(&stage, &["/etc/hosts"], &mut cx);
}

#[gpui_kit::test]
fn files_read_before_the_first_list_is_answered_are_rows_and_leave_the_group_when_the_list_holds_them(
    cx: &mut TestAppContext,
) {
    let t = turn_id('t');
    let (mut stage, mut cx) = in_the_live_diff(cx);
    start_turn(&mut stage, &t, &s0(), &mut cx);
    let first_list = stage.keeper.expect(
        Request::CompareVersions {
            workspace: id('1'),
            from: s0(),
            to: None,
        },
        &mut cx,
    );
    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 1, read("/etc/hosts")));
    stage
        .keeper
        .tell(step_fact(session_id('a'), t.clone(), 2, read("a.txt")));
    wait_for_reads(&stage, &["/etc/hosts", "a.txt"], &mut cx);
    assert_eq!(
        live_rows(&stage, &mut cx),
        [
            PaneRow::Read {
                path: "/etc/hosts".into()
            },
            PaneRow::Read {
                path: "a.txt".into()
            },
        ],
        "before the list is here nothing is taken out of the group"
    );
    stage.keeper.reply(
        first_list,
        Ok(Reply::Compared(vec![file("a.txt", FileStatus::Modified)])),
    );
    answer_reads(
        &mut stage,
        &s0(),
        None,
        &[("a.txt", "one\n", "two\n")],
        &mut cx,
    );
    wait_for_live(&stage, &["a.txt"], &["/etc/hosts"], &mut cx);
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
        now: None,
        state: yaar_link::ThreadState::Sent,
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

fn edit(path: &str, first: u32, last: u32) -> Step {
    Step::Edit {
        path: path.into(),
        lines: Some(std::iter::once(first..=last).collect()),
    }
}

#[gpui_kit::test]
fn an_edit_step_scrolls_and_focuses_the_edited_line_in_the_live_diff(cx: &mut TestAppContext) {
    let t = turn_id('t');
    let (mut stage, mut cx) = in_the_live_diff(cx);
    start_turn(&mut stage, &t, &s0(), &mut cx);
    compared(&mut stage, &s0(), None, Vec::new(), &mut cx);

    let before: String = (1..=100).map(|n| format!("line {n}\n")).collect();
    let after: String = (1..=100)
        .map(|n| match n {
            1..=70 => format!("changed {n}\n"),
            85 => "edited 85\n".to_owned(),
            _ => format!("line {n}\n"),
        })
        .collect();

    stage.keeper.tell(step_fact(
        session_id('a'),
        t.clone(),
        1,
        edit("big.rs", 85, 85),
    ));
    compared(
        &mut stage,
        &s0(),
        None,
        vec![file("big.rs", FileStatus::Modified)],
        &mut cx,
    );
    answer_reads(
        &mut stage,
        &s0(),
        None,
        &[("big.rs", &before, &after)],
        &mut cx,
    );
    wait_for_live(&stage, &["big.rs"], &[], &mut cx);

    let wanted = Position {
        path: "big.rs".into(),
        side: Side::New,
        line: 84,
    };
    wait_until(&mut cx, "focus on the edited line", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.live_position() == Some(&wanted))
    });
    let (rows, edit_row) = stage.pane.read_with(&cx, |pane, _| {
        (pane.live_rows().to_vec(), pane.flow_at_row())
    });
    let edit_row = edit_row.expect("the edit step resolves to a diff row");
    assert_eq!(
        rows[edit_row],
        PaneRow::Diff {
            path: "big.rs".into(),
            row: Row::Added { new: 84 },
        }
    );
    assert!(
        edit_row > 0,
        "focus is on the edited diff row, not the header"
    );
    wait_until(&mut cx, "the edited line scrolled into view", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            let in_view = pane.flow_rows_in_view();
            in_view.contains(&edit_row) && in_view.start > 0
        })
    });
    assert!(drawn(&format!("flow-text-{edit_row}-new:edited 85"), &mut cx).is_some());
    assert!(drawn("flow-row-0", &mut cx).is_none());
}

#[gpui_kit::test]
fn a_second_edit_to_the_same_file_re_scrolls_once_the_updated_diff_loads(cx: &mut TestAppContext) {
    let t = turn_id('t');
    let (mut stage, mut cx) = in_the_live_diff(cx);
    start_turn(&mut stage, &t, &s0(), &mut cx);
    compared(&mut stage, &s0(), None, Vec::new(), &mut cx);

    let before: String = (1..=100).map(|n| format!("line {n}\n")).collect();
    let after_first: String = (1..=100)
        .map(|n| match n {
            1..=70 => format!("first {n}\n"),
            _ => format!("line {n}\n"),
        })
        .collect();
    let after_second: String = (1..=100)
        .map(|n| match n {
            1..=70 => format!("first {n}\n"),
            90 => "second 90\n".to_owned(),
            _ => format!("line {n}\n"),
        })
        .collect();

    stage.keeper.tell(step_fact(
        session_id('a'),
        t.clone(),
        1,
        edit("big.rs", 5, 5),
    ));
    compared(
        &mut stage,
        &s0(),
        None,
        vec![file("big.rs", FileStatus::Modified)],
        &mut cx,
    );
    answer_reads(
        &mut stage,
        &s0(),
        None,
        &[("big.rs", &before, &after_first)],
        &mut cx,
    );
    wait_for_live(&stage, &["big.rs"], &[], &mut cx);

    let first_pos = Position {
        path: "big.rs".into(),
        side: Side::New,
        line: 4,
    };
    wait_until(&mut cx, "focus on the first edited line", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.live_position() == Some(&first_pos))
    });

    // The second edit arrives while the file is already open with the earlier diff; follow must
    // re-scroll once the updated diff loads asynchronously.
    stage.keeper.tell(step_fact(
        session_id('a'),
        t.clone(),
        2,
        edit("big.rs", 90, 90),
    ));
    cx.run_until_parked();
    compared(
        &mut stage,
        &s0(),
        None,
        vec![file("big.rs", FileStatus::Modified)],
        &mut cx,
    );
    cx.run_until_parked();
    answer_reads(
        &mut stage,
        &s0(),
        None,
        &[("big.rs", &before, &after_second)],
        &mut cx,
    );

    let second_pos = Position {
        path: "big.rs".into(),
        side: Side::New,
        line: 89,
    };
    wait_until(&mut cx, "focus moved to the second edited line", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.live_position() == Some(&second_pos))
    });
    let (rows, second_row) = stage.pane.read_with(&cx, |pane, _| {
        (pane.live_rows().to_vec(), pane.flow_at_row())
    });
    let second_row = second_row.expect("the second edit resolves to a diff row");
    assert_eq!(
        rows[second_row],
        PaneRow::Diff {
            path: "big.rs".into(),
            row: Row::Added { new: 89 },
        }
    );
    wait_until(&mut cx, "the second edited line scrolled into view", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            let in_view = pane.flow_rows_in_view();
            in_view.contains(&second_row) && in_view.start > 0
        })
    });
    assert!(drawn(&format!("flow-text-{second_row}-new:second 90"), &mut cx).is_some());
    assert!(drawn("flow-row-0", &mut cx).is_none());
}

#[gpui_kit::test]
fn following_a_read_step_opens_the_file_highlights_its_line_range_and_jumps_across_files(
    cx: &mut TestAppContext,
) {
    use common::navigator::NavigatorEnd;
    use yaar_navigator::{Ask, FileKind, OpenedFile, Told};

    let t = turn_id('t');
    let (mut stage, mut cx) = in_the_live_diff(cx);
    let mut navigator = NavigatorEnd::connect(&stage.models, &mut cx);
    start_turn(&mut stage, &t, &s0(), &mut cx);
    compared(&mut stage, &s0(), None, Vec::new(), &mut cx);

    let first_text: String = (1..=100)
        .map(|n| format!("fn first_{n}() {{ let x = {n}; }}\n"))
        .collect();
    let second_text: String = (1..=60)
        .map(|n| format!("fn second_{n}() {{ let y = {n}; }}\n"))
        .collect();

    stage.keeper.tell(step_fact(
        session_id('a'),
        t.clone(),
        1,
        read_lines("src/first.rs", 40, 45),
    ));
    let (ask_id, loc) = match navigator.ask(&mut cx) {
        Ask::Open { id, location } => (id, location),
        other => panic!("expected Ask::Open, got {other:?}"),
    };
    assert!(loc.path.ends_with("src/first.rs"), "{:?}", loc.path);
    assert_eq!(loc.line, 40);
    navigator.tell(
        Told::Opened {
            id: ask_id,
            file: Ok(OpenedFile {
                path: loc.path,
                kind: FileKind::Worktree {
                    workspace: id('1'),
                    relative: PathBuf::from("src/first.rs"),
                },
                text: first_text,
            }),
        },
        &mut cx,
    );

    wait_until(&mut cx, "first.rs opened in live read view", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.live_reading().is_some_and(|shown| {
                shown.path == std::path::Path::new("src/first.rs")
                    && shown.lines == Some(40..=45)
                    && shown.line_count == 100
            })
        })
    });
    wait_until(&mut cx, "line 40 scrolled into view", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.live_reading_in_view().contains(&39))
    });
    assert!(drawn("flow-live-read-header", &mut cx).is_some());
    assert!(drawn("flow-live-read-path:src/first.rs", &mut cx).is_some());
    assert!(drawn("flow-live-read-active-40", &mut cx).is_some());
    assert!(drawn("flow-live-read-active-45", &mut cx).is_some());
    assert!(drawn("flow-live-read-line-46", &mut cx).is_some());

    // A second read in another file jumps the live view straight to the new file and range.
    stage.keeper.tell(step_fact(
        session_id('a'),
        t,
        2,
        read_lines("src/second.rs", 12, 18),
    ));
    let (ask_id, loc) = match navigator.ask(&mut cx) {
        Ask::Open { id, location } => (id, location),
        other => panic!("expected Ask::Open for second.rs, got {other:?}"),
    };
    assert!(loc.path.ends_with("src/second.rs"), "{:?}", loc.path);
    assert_eq!(loc.line, 12);
    navigator.tell(
        Told::Opened {
            id: ask_id,
            file: Ok(OpenedFile {
                path: loc.path,
                kind: FileKind::Worktree {
                    workspace: id('1'),
                    relative: PathBuf::from("src/second.rs"),
                },
                text: second_text,
            }),
        },
        &mut cx,
    );
    wait_until(&mut cx, "second.rs opened in live read view", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.live_reading().is_some_and(|shown| {
                shown.path == std::path::Path::new("src/second.rs")
                    && shown.lines == Some(12..=18)
                    && shown.line_count == 60
            })
        })
    });
    assert!(drawn("flow-live-read-path:src/second.rs", &mut cx).is_some());
    assert!(drawn("flow-live-read-active-12", &mut cx).is_some());
}

#[gpui_kit::test]
fn an_edit_step_after_a_read_switches_from_the_read_file_to_the_live_diff_centered_on_the_edit(
    cx: &mut TestAppContext,
) {
    use common::navigator::NavigatorEnd;
    use yaar_navigator::{Ask, FileKind, OpenedFile, Told};

    let t = turn_id('t');
    let (mut stage, mut cx) = in_the_live_diff(cx);
    let mut navigator = NavigatorEnd::connect(&stage.models, &mut cx);
    start_turn(&mut stage, &t, &s0(), &mut cx);
    compared(&mut stage, &s0(), None, Vec::new(), &mut cx);

    let before: String = (1..=40).map(|n| format!("line {n}\n")).collect();
    let after: String = (1..=40)
        .map(|n| {
            if n == 20 {
                "edited line 20\n".to_owned()
            } else {
                format!("line {n}\n")
            }
        })
        .collect();

    stage.keeper.tell(step_fact(
        session_id('a'),
        t.clone(),
        1,
        read_lines("src/first.rs", 15, 25),
    ));
    let (ask_id, loc) = match navigator.ask(&mut cx) {
        Ask::Open { id, location } => (id, location),
        other => panic!("expected Ask::Open, got {other:?}"),
    };
    navigator.tell(
        Told::Opened {
            id: ask_id,
            file: Ok(OpenedFile {
                path: loc.path,
                kind: FileKind::Worktree {
                    workspace: id('1'),
                    relative: PathBuf::from("src/first.rs"),
                },
                text: before.clone(),
            }),
        },
        &mut cx,
    );
    wait_until(&mut cx, "first.rs opened", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.live_reading().is_some())
    });

    stage.keeper.tell(step_fact(
        session_id('a'),
        t,
        2,
        edit("src/first.rs", 20, 20),
    ));
    wait_until(
        &mut cx,
        "an edit clears the read file view and switches to the live diff",
        |cx| {
            stage
                .pane
                .read_with(cx, |pane, _| pane.live_reading().is_none())
        },
    );
    compared(
        &mut stage,
        &s0(),
        None,
        vec![file("src/first.rs", FileStatus::Modified)],
        &mut cx,
    );
    answer_reads(
        &mut stage,
        &s0(),
        None,
        &[("src/first.rs", &before, &after)],
        &mut cx,
    );
    let edited = Position {
        path: "src/first.rs".into(),
        side: Side::New,
        line: 19,
    };
    wait_until(&mut cx, "live diff centered on the edited line", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.live_position() == Some(&edited))
    });
}

#[gpui_kit::test]
fn a_command_step_keeps_the_followed_read_file_visible_under_the_activity_banner(
    cx: &mut TestAppContext,
) {
    use common::navigator::NavigatorEnd;
    use yaar_navigator::{Ask, FileKind, OpenedFile, Told};

    let t = turn_id('t');
    let (mut stage, mut cx) = in_the_live_diff(cx);
    let mut navigator = NavigatorEnd::connect(&stage.models, &mut cx);
    start_turn(&mut stage, &t, &s0(), &mut cx);
    compared(&mut stage, &s0(), None, Vec::new(), &mut cx);

    stage.keeper.tell(step_fact(
        session_id('a'),
        t.clone(),
        1,
        read_lines("src/first.rs", 1, 10),
    ));
    let (ask_id, loc) = match navigator.ask(&mut cx) {
        Ask::Open { id, location } => (id, location),
        other => panic!("expected Ask::Open, got {other:?}"),
    };
    navigator.tell(
        Told::Opened {
            id: ask_id,
            file: Ok(OpenedFile {
                path: loc.path,
                kind: FileKind::Worktree {
                    workspace: id('1'),
                    relative: PathBuf::from("src/first.rs"),
                },
                text: "one\ntwo\nthree\n".to_owned(),
            }),
        },
        &mut cx,
    );
    wait_until(&mut cx, "first.rs opened", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.live_reading().is_some())
    });

    stage.keeper.tell(step_fact(
        session_id('a'),
        t,
        2,
        Step::Command {
            command: "cargo test".to_owned(),
        },
    ));
    wait_until(&mut cx, "command step recorded", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.activity().is_some())
    });
    assert!(drawn("flow-activity", &mut cx).is_some());
    assert!(drawn("flow-live-read-header", &mut cx).is_some());
    assert_eq!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.live_reading().map(|r| r.path)),
        Some(PathBuf::from("src/first.rs"))
    );
}

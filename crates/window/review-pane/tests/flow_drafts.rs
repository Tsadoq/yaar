//! Component test: comments drafted in Flow on a line of the live diff.
//! A negative clause is read after a barrier: a later message through the same path, whose arrival
//! shows that everything before it arrived.

mod common;

use std::path::PathBuf;

use gpui_kit::{Modifiers, TestAppContext, VisualTestContext};
use yaar_diff::{Row, Side};
use yaar_link::{
    Anchor, ChangedFile, DraftOn, Fact, FileStatus, FlowForm, Lines, Reply, Request, Session,
    SessionState, Setting, Side as LinkSide, ThreadId, TurnId,
};
use yaar_review_pane::{CommentBox, PaneRow, Position, View};

use common::{
    Stage, agents_of, drawn, file, id, open_staged, review_id, running_turn, session, session_id,
    since_main, step_fact, texts, turn_id, wait_for_files, wait_until, working_on, workspace,
};

fn s0() -> String {
    "5".repeat(40)
}

fn s1() -> String {
    "6".repeat(40)
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

fn changes_rows(stage: &Stage, cx: &mut VisualTestContext) -> Vec<PaneRow> {
    stage.pane.read_with(cx, |pane, _| pane.rows().to_vec())
}

fn comment_box(stage: &Stage, cx: &mut VisualTestContext) -> Option<CommentBox> {
    stage.pane.read_with(cx, |pane, _| pane.comment_box())
}

fn notice(stage: &Stage, cx: &mut VisualTestContext) -> Option<String> {
    stage.pane.read_with(cx, |pane, _| pane.notice())
}

fn index_of(rows: &[PaneRow], path: &str, wanted: Row) -> usize {
    rows.iter()
        .position(|row| {
            matches!(row, PaneRow::Diff { path: at, row } if at == &PathBuf::from(path) && *row == wanted)
        })
        .unwrap_or_else(|| panic!("no row {wanted:?} of {path} in {rows:#?}"))
}

fn wait_for_live(stage: &Stage, wanted: &[&str], cx: &mut VisualTestContext) {
    wait_until(cx, "the live diff's files", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            let rows = pane.live_rows();
            let headings: Vec<String> = rows
                .iter()
                .filter_map(|row| match row {
                    PaneRow::File { path, .. } => Some(path.display().to_string()),
                    _ => None,
                })
                .collect();
            headings == wanted
                && !rows
                    .iter()
                    .any(|row| matches!(row, PaneRow::Unshown { .. }))
        })
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

fn read_of(
    stage: &mut Stage,
    (old, new): (&str, Option<&str>),
    (old_path, path): (&str, &str),
    (before, after): (&str, &str),
    cx: &mut VisualTestContext,
) {
    let asked = stage.keeper.expect(
        Request::ReadVersions {
            workspace: id('1'),
            old: Some(old.to_owned()),
            old_path: old_path.into(),
            new: new.map(str::to_owned),
            path: path.into(),
        },
        cx,
    );
    stage
        .keeper
        .reply(asked, texts(before, after).map(Reply::File));
}

fn open_in(
    form: FlowForm,
    changes: Vec<ChangedFile>,
    sides: (&str, &str),
    cx: &mut TestAppContext,
) -> (Stage, VisualTestContext) {
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature")],
        |keeper| {
            keeper.set_settings(vec![Setting::FlowForm {
                workspace: id('1'),
                form,
            }]);
            keeper.set_agents(vec![agents_of(vec![session_a_idle()])]);
        },
        cx,
    );
    let first = since_main(Vec::new(), changes);
    stage
        .keeper
        .show(&first, |_| texts(sides.0, sides.1), &mut cx);
    wait_for_files(&stage.pane, &mut cx);
    focus_by_the_bar(&stage, &mut cx);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.view()),
        View::Changes
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

fn type_and_save(text: &str, stage: &mut Stage, cx: &mut VisualTestContext) -> Request {
    let mut last = None;
    let mut so_far = String::new();
    for typed in text.chars() {
        so_far.push(typed);
        cx.simulate_input(&typed.to_string());
        let (asked, request) = stage.keeper.request(cx);
        let Request::SaveDraft { text: saved, .. } = &request else {
            panic!("the key {typed:?} was typed, and the Window asked {request:?}");
        };
        assert_eq!(*saved, so_far);
        stage.keeper.reply(asked, Ok(Reply::Done));
        last = Some(request);
    }
    cx.run_until_parked();
    last.expect("some text was typed")
}

fn new_thread(request: &Request) -> (Anchor, ThreadId, yaar_link::CommentId) {
    let Request::SaveDraft {
        review,
        thread,
        comment,
        on: DraftOn::NewThread { anchor },
        ..
    } = request
    else {
        panic!("a save of a new thread was expected, not {request:?}");
    };
    assert_eq!(*review, review_id());
    (anchor.clone(), thread.clone(), comment.clone())
}

fn walk_to(wanted: &Position, stage: &Stage, cx: &mut VisualTestContext) {
    for _ in 0..50 {
        if stage
            .pane
            .read_with(cx, |pane, _| pane.position().cloned())
            .as_ref()
            == Some(wanted)
        {
            return;
        }
        press("j", cx);
    }
    panic!("`j` never came to {wanted:?}");
}

#[gpui_kit::test]
fn a_comment_drafted_on_a_line_in_flow_shows_on_that_line_in_changes_and_is_anchored_as_changes_anchors_it(
    cx: &mut TestAppContext,
) {
    let t = turn_id('t');
    let (mut stage, mut cx) = open_in(
        FlowForm::LiveDiff,
        vec![file("a.txt", FileStatus::Modified)],
        ("one\n", "one\ntwo\n"),
        cx,
    );
    click("review-view-flow", &mut cx);
    start_turn(&mut stage, &t, &s0(), &mut cx);
    compared(
        &mut stage,
        &s0(),
        None,
        vec![file("a.txt", FileStatus::Modified)],
        &mut cx,
    );
    read_of(
        &mut stage,
        (&s0(), None),
        ("a.txt", "a.txt"),
        ("one\n", "one\ntwo\n"),
        &mut cx,
    );
    wait_for_live(&stage, &["a.txt"], &mut cx);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.flow_form()),
        FlowForm::LiveDiff
    );

    let two = index_of(&live_rows(&stage, &mut cx), "a.txt", Row::Added { new: 1 });
    click(&format!("flow-text-{two}-new:two"), &mut cx);
    assert_eq!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.live_position().cloned()),
        Some(Position {
            path: "a.txt".into(),
            side: Side::New,
            line: 1,
        })
    );
    press("c", &mut cx);
    let opened = comment_box(&stage, &mut cx).expect("a comment box opened on the line");
    assert_eq!(opened.at.line, 1);
    assert!(drawn("review-comment-box", &mut cx).is_some());
    let wanted = Anchor {
        tree: None,
        path: "a.txt".into(),
        side: LinkSide::New,
        lines: Lines { first: 2, last: 2 },
        text: "two".to_owned(),
    };
    assert_eq!(opened.anchor, Some(wanted.clone()));
    let saved = type_and_save("why?", &mut stage, &mut cx);
    let (first_anchor, thread, comment) = new_thread(&saved);
    assert_eq!(first_anchor, wanted);
    let Request::SaveDraft { text, .. } = &saved else {
        unreachable!("new_thread checked it");
    };
    assert_eq!(text, "why?");

    let rows = live_rows(&stage, &mut cx);
    let two = index_of(&rows, "a.txt", Row::Added { new: 1 });
    assert_eq!(
        rows[two + 1..two + 3],
        [
            PaneRow::Thread {
                thread: thread.clone()
            },
            PaneRow::Comment {
                thread: thread.clone(),
                comment: comment.clone()
            }
        ]
    );
    press("escape", &mut cx);
    assert_eq!(comment_box(&stage, &mut cx), None);
    assert!(drawn(&format!("flow-thread-{thread}"), &mut cx).is_some());
    assert!(drawn(&format!("flow-comment-{comment}:draft"), &mut cx).is_some());

    click("review-view-changes", &mut cx);
    let rows = changes_rows(&stage, &mut cx);
    let two = index_of(&rows, "a.txt", Row::Added { new: 1 });
    assert_eq!(
        rows[two + 1..two + 3],
        [
            PaneRow::Thread {
                thread: thread.clone()
            },
            PaneRow::Comment {
                thread: thread.clone(),
                comment: comment.clone()
            }
        ]
    );
    assert!(drawn(&format!("review-comment-{comment}:draft"), &mut cx).is_some());

    walk_to(
        &Position {
            path: "a.txt".into(),
            side: Side::New,
            line: 1,
        },
        &stage,
        &mut cx,
    );
    press("c", &mut cx);
    let again = type_and_save("also", &mut stage, &mut cx);
    let (second_anchor, second_thread, _) = new_thread(&again);
    assert_eq!(second_anchor, first_anchor);
    assert_ne!(second_thread, thread);
    press("escape", &mut cx);

    // `shift-t` keeps the selection, so Changes asks nothing when the turn ends with a state.
    // A draft in Flow then names that state as the tree of the new side.
    click("review-view-flow", &mut cx);
    press("shift-t", &mut cx);
    stage.keeper.tell(Fact::TurnEnded {
        workspace: id('1'),
        session: session_id('a'),
        turn: t.clone(),
        state: Some(s1()),
        since: None,
    });
    compared(
        &mut stage,
        &s0(),
        Some(&s1()),
        vec![file("a.txt", FileStatus::Modified)],
        &mut cx,
    );
    read_of(
        &mut stage,
        (&s0(), Some(&s1())),
        ("a.txt", "a.txt"),
        ("one\n", "one\ntwo\n"),
        &mut cx,
    );
    wait_for_live(&stage, &["a.txt"], &mut cx);
    let two = index_of(&live_rows(&stage, &mut cx), "a.txt", Row::Added { new: 1 });
    click(&format!("flow-text-{two}-new:two"), &mut cx);
    press("c", &mut cx);
    let ended = type_and_save("x", &mut stage, &mut cx);
    let (ended_anchor, ..) = new_thread(&ended);
    assert_eq!(
        ended_anchor,
        Anchor {
            tree: Some(s1()),
            ..wanted
        }
    );
}

#[gpui_kit::test]
fn a_comment_on_a_removed_line_in_flow_is_anchored_in_the_version_the_turn_started_from_under_the_name_the_file_had_there(
    cx: &mut TestAppContext,
) {
    let t = turn_id('t');
    let (mut stage, mut cx) = open_in(FlowForm::LiveDiff, Vec::new(), ("", ""), cx);
    click("review-view-flow", &mut cx);
    start_turn(&mut stage, &t, &s0(), &mut cx);
    let renamed = ChangedFile {
        path: "new name.txt".into(),
        status: FileStatus::Renamed {
            from: "old name.txt".into(),
        },
    };
    compared(&mut stage, &s0(), None, vec![renamed], &mut cx);
    read_of(
        &mut stage,
        (&s0(), None),
        ("old name.txt", "new name.txt"),
        ("one\ntwo\n", "one\nTWO\n"),
        &mut cx,
    );
    wait_for_live(&stage, &["new name.txt"], &mut cx);

    let rows = live_rows(&stage, &mut cx);
    let removed = index_of(&rows, "new name.txt", Row::Removed { old: 1 });
    click(&format!("flow-text-{removed}-old:two"), &mut cx);
    press("c", &mut cx);
    let saved = type_and_save("a", &mut stage, &mut cx);
    assert_eq!(
        new_thread(&saved).0,
        Anchor {
            tree: Some(s0()),
            path: "old name.txt".into(),
            side: LinkSide::Old,
            lines: Lines { first: 2, last: 2 },
            text: "two".to_owned(),
        }
    );
    press("escape", &mut cx);

    let rows = live_rows(&stage, &mut cx);
    let added = index_of(&rows, "new name.txt", Row::Added { new: 1 });
    click(&format!("flow-text-{added}-new:TWO"), &mut cx);
    press("c", &mut cx);
    let saved = type_and_save("b", &mut stage, &mut cx);
    assert_eq!(
        new_thread(&saved).0,
        Anchor {
            tree: None,
            path: "new name.txt".into(),
            side: LinkSide::New,
            lines: Lines { first: 2, last: 2 },
            text: "TWO".to_owned(),
        }
    );
}

#[gpui_kit::test]
fn c_in_flow_writes_nothing_where_there_is_no_line_of_the_live_diff_to_write_on(
    cx: &mut TestAppContext,
) {
    let t = turn_id('t');
    // Changes holds a file with a line to stand on, so a box opened from where Changes has the
    // reviewer would show.
    let (mut stage, mut cx) = open_in(
        FlowForm::Timeline,
        vec![file("a.txt", FileStatus::Modified)],
        ("one\n", "one\ntwo\n"),
        cx,
    );
    assert!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.position().is_some()),
        "Changes puts the reviewer on a line"
    );
    click("review-view-flow", &mut cx);
    start_turn(&mut stage, &t, &s0(), &mut cx);

    press("c", &mut cx);
    assert_eq!(
        notice(&stage, &mut cx).as_deref(),
        Some("A comment is written on a line of the live diff")
    );
    assert_eq!(comment_box(&stage, &mut cx), None);

    press("shift-l", &mut cx);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.flow_form()),
        FlowForm::LiveDiff
    );
    compared(&mut stage, &s0(), None, Vec::new(), &mut cx);
    stage.keeper.tell(step_fact(
        session_id('a'),
        t,
        1,
        yaar_link::Step::Read {
            path: "r.txt".into(),
            lines: None,
        },
    ));
    wait_until(&mut cx, "the file read", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.live_rows()
                .iter()
                .any(|row| matches!(row, PaneRow::Read { .. }))
        })
    });
    press("c", &mut cx);
    assert_eq!(
        notice(&stage, &mut cx).as_deref(),
        Some("The reviewer is on no line")
    );
    assert_eq!(comment_box(&stage, &mut cx), None);

    // The barrier: a draft is saved only on typed text, so the missing box asserted above is what
    // shows that no comment was begun.
    stage
        .keeper
        .tell(Fact::ChangesChanged { workspace: id('1') });
    let (_, request) = stage.keeper.request(&mut cx);
    assert!(
        matches!(
            request,
            Request::GetChanges { .. } | Request::CompareVersions { .. }
        ),
        "the next request is {request:?}"
    );
}

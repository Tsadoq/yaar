//! Component test: the tree of Changes in the Review pane on GPUI's test platform. Keys,
//! clicks and the wheel go in; the pane is read back through the model it shows. The test
//! plays the Keeper on the other end of a socket pair: it sets the history and the
//! uncommitted changes the Keeper would answer with, and answers the comparisons and the
//! files the pane asks for.

mod common;

use std::path::PathBuf;

use gpui_kit::{Bounds, Entity, Modifiers, Pixels, TestAppContext, VisualTestContext};
use yaar_link::{
    ChangedFile, CreditedFile, Fact, FileStatus, History, HistoryEntry, Reply, Request, RequestId,
    SessionId, TurnId,
};
use yaar_review_pane::{ChangeShown, PaneRow, ReviewPane};

use common::{
    COMMIT, Stage, commit, drawn, file, id, link_up, no_agent_review, open_window, repository,
    review_id, rows, since_main, texts, wait_for_files, wait_until, workspace,
};

/// The id of a commit or a state that is this digit forty times.
fn version(digit: char) -> String {
    digit.to_string().repeat(40)
}

/// Where the history of the tests starts: the commit the changes are counted from.
fn start() -> String {
    COMMIT.to_owned()
}

/// The first commit of the history of the tests.
fn c1() -> String {
    version('1')
}

/// The second commit of the history of the tests, after the turn.
fn c2() -> String {
    version('2')
}

/// The state recorded at the end of the turn of the history of the tests.
fn s1() -> String {
    version('5')
}

/// The session that made the turn, and the one change of the uncommitted list.
fn session() -> SessionId {
    SessionId::new("5".repeat(32))
}

fn turn(digit: char, state: Option<String>) -> HistoryEntry {
    HistoryEntry::Turn {
        turn: TurnId::new(digit.to_string().repeat(32)),
        session: session(),
        state,
    }
}

/// Commits C1 and C2 and, between them, the turn T1.
fn history() -> History {
    History {
        workspace: id('1'),
        start: Some(start()),
        entries: vec![
            HistoryEntry::Commit(commit('1', "First")),
            turn('a', Some(s1())),
            HistoryEntry::Commit(commit('2', "Second")),
        ],
    }
}

/// What the Keeper lists as uncommitted: a change of the session, and one no agent made.
fn uncommitted() -> Vec<CreditedFile> {
    vec![
        CreditedFile {
            file: file("mine.txt", FileStatus::Modified),
            by: vec![session()],
        },
        CreditedFile {
            file: file("outside.txt", FileStatus::Added),
            by: Vec::new(),
        },
    ]
}

/// A window with the pane in it that shows the changes of workspace `1` against its
/// base, with one file, and then learns `history` and `uncommitted`. The history is set
/// after the changes are answered, since a pane whose changes are not yet here shows the
/// last recorded turn instead of them.
fn open_with_history(
    history: History,
    uncommitted: Vec<CreditedFile>,
    cx: &mut TestAppContext,
) -> (Stage, VisualTestContext) {
    let (models, keeper) = link_up(vec![workspace('1', "feature")], cx);
    keeper.set_uncommitted(uncommitted);
    let (mut stage, mut cx) = open_window(models, keeper, cx);
    let changes = since_main(
        vec![commit('b', "Edit the story")],
        vec![file("story.txt", FileStatus::Modified)],
    );
    stage
        .keeper
        .show(&changes, |_| texts("a\nb\n", "a\nc\n"), &mut cx);
    wait_for_files(&stage.pane, &mut cx);
    let asked = stage.keeper.history_asked();
    stage.keeper.set_history(history.clone());
    stage
        .keeper
        .tell(Fact::Workspaces(vec![repository(vec![workspace(
            '1', "feature",
        )])]));
    wait_until(&mut cx, "the history in the pane", |cx| {
        stage.keeper.history_asked() > asked
            && stage
                .pane
                .read_with(cx, |pane, _| pane.history() == Some(&history))
    });
    wait_until(
        &mut cx,
        "the uncommitted changes of the review shown",
        |cx| {
            stage.keeper.uncommitted_review() == Some(review_id())
                && stage
                    .pane
                    .read_with(cx, |pane, _| pane.uncommitted().is_some())
        },
    );
    (stage, cx)
}

/// Where the entry `i` of the tree is drawn, or the uncommitted one with `None`.
fn entry_bounds(entry: Option<usize>, cx: &mut VisualTestContext) -> Option<Bounds<Pixels>> {
    match entry {
        Some(i) => drawn(&format!("review-history-{i}"), cx),
        None => drawn("review-history-uncommitted", cx),
    }
}

fn click_entry(entry: Option<usize>, modifiers: Modifiers, cx: &mut VisualTestContext) {
    let at = entry_bounds(entry, cx)
        .unwrap_or_else(|| panic!("entry {entry:?} of the tree is not drawn"))
        .center();
    cx.simulate_click(at, modifiers);
}

fn shown(pane: &Entity<ReviewPane>, cx: &mut VisualTestContext) -> ChangeShown {
    pane.read_with(cx, |pane, _| pane.change_shown())
}

/// Answers the comparison the pane asks for with these files.
fn compared(
    stage: &mut Stage,
    from: String,
    to: Option<String>,
    files: Vec<ChangedFile>,
    cx: &mut VisualTestContext,
) {
    let asked = stage.keeper.expect(
        Request::CompareVersions {
            workspace: id('1'),
            from,
            to,
        },
        cx,
    );
    stage.keeper.reply(asked, Ok(Reply::Compared(files)));
}

/// Answers the read of the two versions of a file with these texts.
fn read_versions(
    stage: &mut Stage,
    request: Request,
    old: &str,
    new: &str,
    cx: &mut VisualTestContext,
) {
    let asked: RequestId = stage.keeper.expect(request, cx);
    stage.keeper.reply(asked, texts(old, new).map(Reply::File));
}

fn versions_of(old: Option<String>, old_path: &str, new: Option<String>, path: &str) -> Request {
    Request::ReadVersions {
        workspace: id('1'),
        old,
        old_path: PathBuf::from(old_path),
        new,
        path: PathBuf::from(path),
    }
}

/// The paths of the files the pane lists, in order.
fn paths(pane: &Entity<ReviewPane>, cx: &mut TestAppContext) -> Vec<String> {
    rows(pane, cx)
        .into_iter()
        .filter_map(|row| match row {
            PaneRow::File { path, .. } => Some(path.display().to_string()),
            _ => None,
        })
        .collect()
}

/// Waits until the pane lists these files with their lines read.
fn wait_for_paths(stage: &Stage, wanted: &[&str], cx: &mut VisualTestContext) {
    wait_until(cx, "the files of the change shown", |cx| {
        let held = paths(&stage.pane, cx);
        held == wanted
            && !rows(&stage.pane, cx).iter().any(|row| {
                matches!(
                    row,
                    PaneRow::Unshown {
                        why: yaar_review_pane::Unshown::Loading,
                        ..
                    }
                )
            })
    });
}

/// Puts the focus on the rows of the pane, where the keys of the pane work.
fn focus_the_rows(stage: &Stage, cx: &mut VisualTestContext) {
    cx.update(|window, cx| stage.focus.focus(window, cx));
}

#[gpui_kit::test]
fn changes_shows_the_tree_of_commits_and_turns_the_uncommitted_changes_with_those_no_agent_made_and_the_files_of_the_selected_change(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with_history(history(), uncommitted(), cx);

    // The tree of commits and turns of the worktree: its history, drawn in its order,
    // the uncommitted changes last.
    let held = stage.pane.read_with(&cx, |pane, _| pane.history().cloned());
    assert_eq!(held, Some(history()));
    let places: Vec<Bounds<Pixels>> = (0..3)
        .map(|i| {
            entry_bounds(Some(i), &mut cx)
                .unwrap_or_else(|| panic!("entry {i} of the tree is not drawn"))
        })
        .collect();
    let last = entry_bounds(None, &mut cx).expect("the uncommitted entry is drawn");
    assert!(places[0].bottom() <= places[1].top(), "{places:?}");
    assert!(places[1].bottom() <= places[2].top(), "{places:?}");
    assert!(places[2].bottom() <= last.top(), "{places:?} {last:?}");
    assert_eq!(entry_bounds(Some(3), &mut cx), None);

    // The uncommitted changes the pane lists are those of the review it shows.
    assert_eq!(stage.keeper.uncommitted_review(), Some(review_id()));
    assert_eq!(no_agent_review().id, review_id());
    let listed = stage
        .pane
        .read_with(&cx, |pane, _| pane.uncommitted().map(<[_]>::to_vec));
    assert_eq!(listed, Some(uncommitted()));

    // Until the reviewer selects, the comparison is shown.
    assert_eq!(shown(&stage.pane, &mut cx), ChangeShown::Comparison);

    // The uncommitted entry shows the files of that list, from the commit checked out to
    // the files on disk, and marks the change no agent made.
    click_entry(None, Modifiers::none(), &mut cx);
    read_versions(
        &mut stage,
        versions_of(Some(c2()), "mine.txt", None, "mine.txt"),
        "one\n",
        "two\n",
        &mut cx,
    );
    read_versions(
        &mut stage,
        versions_of(Some(c2()), "outside.txt", None, "outside.txt"),
        "",
        "new\n",
        &mut cx,
    );
    wait_for_paths(&stage, &["mine.txt", "outside.txt"], &mut cx);
    assert_eq!(shown(&stage.pane, &mut cx), ChangeShown::Uncommitted);
    let heading = |path: &str, cx: &mut VisualTestContext| {
        rows(&stage.pane, cx)
            .iter()
            .position(|row| matches!(row, PaneRow::File { path: held, .. } if held == &PathBuf::from(path)))
            .unwrap_or_else(|| panic!("no heading for {path}"))
    };
    let (mine, outside) = (
        heading("mine.txt", &mut cx),
        heading("outside.txt", &mut cx),
    );
    let made_by = |path: &str, cx: &mut VisualTestContext| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.made_by_no_agent(&PathBuf::from(path)))
    };
    assert!(!made_by("mine.txt", &mut cx));
    assert!(made_by("outside.txt", &mut cx));
    assert!(
        drawn(&format!("review-no-agent-{outside}"), &mut cx).is_some(),
        "the change no agent made is not marked"
    );
    assert!(
        drawn(&format!("review-no-agent-{mine}"), &mut cx).is_none(),
        "the change of the session is marked as no agent's"
    );

    // The files of the selected change: a turn is the change from the version before it
    // to its state.
    click_entry(Some(1), Modifiers::none(), &mut cx);
    compared(
        &mut stage,
        c1(),
        Some(s1()),
        vec![file("turn.txt", FileStatus::Modified)],
        &mut cx,
    );
    read_versions(
        &mut stage,
        versions_of(Some(c1()), "turn.txt", Some(s1()), "turn.txt"),
        "before\n",
        "after\n",
        &mut cx,
    );
    wait_for_paths(&stage, &["turn.txt"], &mut cx);
    assert_eq!(
        shown(&stage.pane, &mut cx),
        ChangeShown::Entries { first: 1, last: 1 }
    );
    let marked = (0..rows(&stage.pane, &mut cx).len())
        .any(|row| drawn(&format!("review-no-agent-{row}"), &mut cx).is_some());
    assert!(!marked, "only the uncommitted changes are marked");
    let lines: Vec<String> = stage.pane.read_with(&cx, |pane, _| {
        pane.rows()
            .iter()
            .filter_map(|row| match row {
                PaneRow::Diff { path, row } => Some(format!("{}:{row:?}", path.display())),
                _ => None,
            })
            .collect()
    });
    assert!(
        lines.iter().any(|line| line.starts_with("turn.txt")),
        "the file of the turn shows no lines: {lines:?}"
    );
}

#[gpui_kit::test]
fn a_click_on_one_commit_shows_the_change_from_the_entry_above_it_to_that_commit(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with_history(history(), uncommitted(), cx);
    click_entry(Some(2), Modifiers::none(), &mut cx);
    compared(
        &mut stage,
        s1(),
        Some(c2()),
        vec![file("second.txt", FileStatus::Added)],
        &mut cx,
    );
    read_versions(
        &mut stage,
        versions_of(Some(s1()), "second.txt", Some(c2()), "second.txt"),
        "",
        "x\n",
        &mut cx,
    );
    wait_for_paths(&stage, &["second.txt"], &mut cx);
    assert_eq!(
        shown(&stage.pane, &mut cx),
        ChangeShown::Entries { first: 2, last: 2 }
    );
}

#[gpui_kit::test]
fn tab_gives_the_tree_the_focus_with_the_cursor_on_the_first_entry_and_enter_shows_the_turn_under_it(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with_history(history(), uncommitted(), cx);
    let at_first = stage.pane.read_with(&cx, |pane, _| {
        (pane.history_cursor(), pane.position().cloned())
    });
    assert_eq!(at_first.0, None, "the tree has not had the focus yet");
    focus_the_rows(&stage, &mut cx);
    cx.simulate_keystrokes("tab");
    let cursor = |stage: &Stage, cx: &mut VisualTestContext| {
        stage.pane.read_with(cx, |pane, _| pane.history_cursor())
    };
    assert_eq!(cursor(&stage, &mut cx), Some(0));
    cx.simulate_keystrokes("j");
    assert_eq!(cursor(&stage, &mut cx), Some(1));
    // The keys of the tree do not move the reviewer in the rows.
    let position = stage
        .pane
        .read_with(&cx, |pane, _| pane.position().cloned());
    assert_eq!(position, at_first.1);
    cx.simulate_keystrokes("enter");
    compared(
        &mut stage,
        c1(),
        Some(s1()),
        vec![file("turn.txt", FileStatus::Modified)],
        &mut cx,
    );
    read_versions(
        &mut stage,
        versions_of(Some(c1()), "turn.txt", Some(s1()), "turn.txt"),
        "a\n",
        "b\n",
        &mut cx,
    );
    wait_for_paths(&stage, &["turn.txt"], &mut cx);
    assert_eq!(
        shown(&stage.pane, &mut cx),
        ChangeShown::Entries { first: 1, last: 1 }
    );
}

#[gpui_kit::test]
fn marked_entries_are_shown_as_a_range_from_the_first_to_the_last_and_a_shift_click_extends_the_one_shown(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with_history(history(), uncommitted(), cx);
    focus_the_rows(&stage, &mut cx);
    cx.simulate_keystrokes("tab j");
    // `k` back to the first entry, mark it, and the turn after it.
    cx.simulate_keystrokes("k space j space");
    let marked = |stage: &Stage, cx: &mut VisualTestContext| {
        stage.pane.read_with(cx, |pane, _| pane.marked_changes())
    };
    assert_eq!(marked(&stage, &mut cx), [0, 1]);
    cx.simulate_keystrokes("enter");
    compared(
        &mut stage,
        start(),
        Some(s1()),
        vec![file("range.txt", FileStatus::Added)],
        &mut cx,
    );
    read_versions(
        &mut stage,
        versions_of(Some(start()), "range.txt", Some(s1()), "range.txt"),
        "",
        "r\n",
        &mut cx,
    );
    wait_for_paths(&stage, &["range.txt"], &mut cx);
    assert_eq!(
        shown(&stage.pane, &mut cx),
        ChangeShown::Entries { first: 0, last: 1 }
    );
    assert_eq!(marked(&stage, &mut cx), Vec::<usize>::new());

    // A click on the first commit, then a click with Shift on the second.
    click_entry(Some(0), Modifiers::none(), &mut cx);
    compared(&mut stage, start(), Some(c1()), vec![], &mut cx);
    wait_for_paths(&stage, &[], &mut cx);
    assert_eq!(
        shown(&stage.pane, &mut cx),
        ChangeShown::Entries { first: 0, last: 0 }
    );
    click_entry(Some(2), Modifiers::shift(), &mut cx);
    compared(
        &mut stage,
        start(),
        Some(c2()),
        vec![file("all.txt", FileStatus::Added)],
        &mut cx,
    );
    read_versions(
        &mut stage,
        versions_of(Some(start()), "all.txt", Some(c2()), "all.txt"),
        "",
        "all\n",
        &mut cx,
    );
    wait_for_paths(&stage, &["all.txt"], &mut cx);
    assert_eq!(
        shown(&stage.pane, &mut cx),
        ChangeShown::Entries { first: 0, last: 2 }
    );
}

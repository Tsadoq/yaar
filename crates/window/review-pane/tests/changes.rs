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
    ReviewId, SessionId, Setting, TurnEndSelects, TurnId, WorkspaceReviews,
};
use yaar_review_pane::{ChangeShown, EXPIRED_TURN, PaneRow, ReviewPane};

use common::{
    COMMIT, Stage, commit, drawn, file, id, link_up, link_up_staged, no_agent_review, open_window,
    read_file, repository, review_id, review_with, reviews_of, rows, since_main, texts,
    wait_for_files, wait_until, workspace,
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
    open_with_reviews(history, uncommitted, None, review_id(), cx)
}

/// As [`open_with_history`], for a Keeper that holds these reviews instead of the one
/// review of no agent, the first of which, `shown`, the pane shows.
fn open_with_reviews(
    history: History,
    uncommitted: Vec<CreditedFile>,
    reviews: Option<WorkspaceReviews>,
    shown: ReviewId,
    cx: &mut TestAppContext,
) -> (Stage, VisualTestContext) {
    let (models, keeper) = link_up(vec![workspace('1', "feature")], cx);
    keeper.set_uncommitted(uncommitted);
    if let Some(reviews) = reviews {
        keeper.set_reviews(reviews);
    }
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
            stage.keeper.uncommitted_review() == Some(shown.clone())
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
    // The turn changed `outside.txt` too, which is also a change no agent made in the
    // uncommitted list: the mark is for the uncommitted entry only.
    click_entry(Some(1), Modifiers::none(), &mut cx);
    compared(
        &mut stage,
        c1(),
        Some(s1()),
        vec![
            file("turn.txt", FileStatus::Modified),
            file("outside.txt", FileStatus::Added),
        ],
        &mut cx,
    );
    read_versions(
        &mut stage,
        versions_of(Some(c1()), "turn.txt", Some(s1()), "turn.txt"),
        "before\n",
        "after\n",
        &mut cx,
    );
    read_versions(
        &mut stage,
        versions_of(Some(c1()), "outside.txt", Some(s1()), "outside.txt"),
        "",
        "new\n",
        &mut cx,
    );
    wait_for_paths(&stage, &["turn.txt", "outside.txt"], &mut cx);
    assert_eq!(
        shown(&stage.pane, &mut cx),
        ChangeShown::Entries { first: 1, last: 1 }
    );
    assert!(
        stage.pane.read_with(&cx, |pane, _| pane
            .made_by_no_agent(&PathBuf::from("outside.txt"))),
        "the premise: the path is one no agent made in the uncommitted list"
    );
    let outside = rows(&stage.pane, &mut cx)
        .iter()
        .position(|row| matches!(row, PaneRow::File { path, .. } if path == &PathBuf::from("outside.txt")))
        .expect("the heading of outside.txt");
    assert!(
        drawn(&format!("review-no-agent-{outside}"), &mut cx).is_none(),
        "the heading of a file of a turn is marked as no agent's"
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

    // A click with Shift above the one shown reaches up to it, the other way round.
    click_entry(Some(2), Modifiers::none(), &mut cx);
    compared(&mut stage, s1(), Some(c2()), vec![], &mut cx);
    wait_for_paths(&stage, &[], &mut cx);
    click_entry(Some(1), Modifiers::shift(), &mut cx);
    compared(&mut stage, c1(), Some(c2()), vec![], &mut cx);
    wait_for_paths(&stage, &[], &mut cx);
    assert_eq!(
        shown(&stage.pane, &mut cx),
        ChangeShown::Entries { first: 1, last: 2 }
    );
}

/// Selects the turn of the history of the tests with a click, and answers what the pane
/// asks for it: one file, `turn.txt`, from `before` to `after`.
fn select_the_turn(stage: &mut Stage, before: &str, after: &str, cx: &mut VisualTestContext) {
    click_entry(Some(1), Modifiers::none(), cx);
    compared(
        stage,
        c1(),
        Some(s1()),
        vec![file("turn.txt", FileStatus::Modified)],
        cx,
    );
    read_versions(
        stage,
        versions_of(Some(c1()), "turn.txt", Some(s1()), "turn.txt"),
        before,
        after,
        cx,
    );
    wait_for_paths(stage, &["turn.txt"], cx);
}

#[gpui_kit::test]
fn a_turn_with_no_state_is_drawn_and_cannot_be_selected_or_marked(cx: &mut TestAppContext) {
    let history = History {
        entries: vec![
            HistoryEntry::Commit(commit('1', "First")),
            turn('a', None),
            HistoryEntry::Commit(commit('2', "Second")),
        ],
        ..history()
    };
    let (mut stage, mut cx) = open_with_history(history, uncommitted(), cx);
    assert!(
        entry_bounds(Some(1), &mut cx).is_some(),
        "the turn is drawn"
    );
    // The click puts the cursor on the turn and the focus in the tree, where the keys that
    // mark and show the entry under the cursor have nothing to mark or show.
    click_entry(Some(1), Modifiers::none(), &mut cx);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.history_cursor()),
        Some(1)
    );
    cx.simulate_keystrokes("space enter");
    let held = stage
        .pane
        .read_with(&cx, |pane, _| (pane.marked_changes(), pane.change_shown()));
    assert_eq!(held, (Vec::new(), ChangeShown::Comparison));

    // A change that is from the version above the turn, and not from the turn: C2 is
    // counted from C1, since the turn has no state to count from. The request that
    // follows is a barrier for the keys and the click above, which asked nothing.
    click_entry(Some(2), Modifiers::none(), &mut cx);
    compared(&mut stage, c1(), Some(c2()), vec![], &mut cx);
    wait_for_paths(&stage, &[], &mut cx);
    assert_eq!(
        shown(&stage.pane, &mut cx),
        ChangeShown::Entries { first: 2, last: 2 }
    );
}

#[gpui_kit::test]
fn the_keys_of_the_tree_do_not_reach_the_rows_and_tab_gives_the_rows_the_focus_back(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with_history(history(), uncommitted(), cx);
    focus_the_rows(&stage, &mut cx);
    cx.simulate_keystrokes("tab");
    let layout = stage.pane.read_with(&cx, |pane, _| pane.layout());
    // `s` switches the layout and `c` opens a comment box, where the rows have the focus.
    cx.simulate_keystrokes("s c y m r");
    let held = stage.pane.read_with(&cx, |pane, _| {
        (pane.layout(), pane.comment_box(), pane.notice())
    });
    assert_eq!((held.0, held.1, held.2), (layout, None, None));
    cx.simulate_keystrokes("tab");
    cx.simulate_keystrokes("s");
    assert_ne!(
        stage.pane.read_with(&cx, |pane, _| pane.layout()),
        layout,
        "the keys of the rows work again"
    );
    cx.update(|window, _| assert!(stage.focus.is_focused(window)));
    // And a key of the tree does nothing where the rows have the focus.
    cx.simulate_keystrokes("space");
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.marked_changes()),
        Vec::<usize>::new()
    );
    let _ = &mut stage;
}

#[gpui_kit::test]
fn the_cursor_stays_in_the_tree_and_the_wheel_scrolls_a_long_history_in_its_own_list(
    cx: &mut TestAppContext,
) {
    let long = History {
        entries: (0..12u8)
            .map(|n| HistoryEntry::Commit(commit(char::from(b'a' + n), &format!("Commit {n}"))))
            .collect(),
        ..history()
    };
    let (stage, mut cx) = open_with_history(long, uncommitted(), cx);
    assert!(entry_bounds(Some(7), &mut cx).is_some());
    assert!(
        entry_bounds(Some(8), &mut cx).is_none(),
        "only 8 entries are in view"
    );
    let rows_in_view = stage.pane.read_with(&cx, |pane, _| pane.rows_in_view());
    let over = entry_bounds(Some(3), &mut cx).expect("drawn").center();
    cx.simulate_event(gpui_kit::ScrollEvent {
        position: over,
        delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
            gpui_kit::px(0.),
            gpui_kit::px(-400.),
        )),
        ..gpui_kit::ScrollEvent::default()
    });
    cx.run_until_parked();
    assert!(
        entry_bounds(Some(0), &mut cx).is_none(),
        "the tree scrolled"
    );
    assert!(
        entry_bounds(None, &mut cx).is_some(),
        "the uncommitted entry is reached"
    );
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.rows_in_view()),
        rows_in_view,
        "the wheel over the tree moved the tree and not the files"
    );

    // The keys take the view along with the cursor.
    focus_the_rows(&stage, &mut cx);
    cx.simulate_keystrokes("tab");
    for _ in 0..13 {
        cx.simulate_keystrokes("j");
    }
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.history_cursor()),
        Some(12),
        "the cursor stops at the uncommitted entry"
    );
    cx.simulate_keystrokes("k");
    let cursor = stage.pane.read_with(&cx, |pane, _| pane.history_cursor());
    assert_eq!(cursor, Some(11));
    assert!(entry_bounds(Some(11), &mut cx).is_some());
}

#[gpui_kit::test]
fn a_selection_stays_when_the_comparison_is_read_again_and_the_uncommitted_changes_are_asked_for_again(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with_history(history(), uncommitted(), cx);
    select_the_turn(&mut stage, "before\n", "after\n", &mut cx);
    let asked = stage.keeper.uncommitted_asked();
    stage
        .keeper
        .tell(Fact::ChangesChanged { workspace: id('1') });
    wait_until(&mut cx, "the uncommitted changes asked for again", |_| {
        stage.keeper.uncommitted_asked() > asked
    });
    // The comparison is read again, and the reviewer's selection stays where it is.
    let changes = since_main(
        vec![commit('b', "Edit the story")],
        vec![file("story.txt", FileStatus::Modified)],
    );
    let again = stage
        .keeper
        .expect(Request::GetChanges { workspace: id('1') }, &mut cx);
    stage.keeper.reply(again, Ok(Reply::Changes(changes)));
    wait_until(&mut cx, "the comparison answered", |cx| {
        stage
            .models
            .changes
            .read_with(cx, |changes, _| !changes.is_loading())
    });
    assert_eq!(
        shown(&stage.pane, &mut cx),
        ChangeShown::Entries { first: 1, last: 1 }
    );
    assert_eq!(paths(&stage.pane, &mut cx), ["turn.txt"]);
    // Nothing was asked for the files of the comparison: another workspace is the next
    // thing the Window asks about.
    stage
        .keeper
        .tell(Fact::Workspaces(vec![repository(vec![workspace(
            '2', "other",
        )])]));
    let next = stage
        .keeper
        .expect(Request::GetChanges { workspace: id('2') }, &mut cx);
    stage.keeper.reply(
        next,
        Ok(Reply::Changes(yaar_link::Changes {
            workspace: id('2'),
            ..since_main(Vec::new(), Vec::new())
        })),
    );
}

#[gpui_kit::test]
fn the_history_is_asked_for_again_at_workspaces_and_at_a_turn_end_and_the_selection_is_kept_when_its_entries_are_as_they_were(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with_history(history(), uncommitted(), cx);
    select_the_turn(&mut stage, "before\n", "after\n", &mut cx);

    // A commit joins the history: the turn is still entry 1 and what it leads to has not
    // changed, so nothing is asked again for its files.
    let grown = History {
        entries: {
            let mut entries = history().entries;
            entries.push(HistoryEntry::Commit(commit('3', "Third")));
            entries
        },
        ..history()
    };
    let asked = stage.keeper.history_asked();
    stage.keeper.set_history(grown.clone());
    stage
        .keeper
        .tell(Fact::Workspaces(vec![repository(vec![workspace(
            '1', "feature",
        )])]));
    wait_until(&mut cx, "the longer history", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.history() == Some(&grown))
    });
    assert!(stage.keeper.history_asked() > asked);
    assert!(entry_bounds(Some(3), &mut cx).is_some());
    assert_eq!(
        shown(&stage.pane, &mut cx),
        ChangeShown::Entries { first: 1, last: 1 }
    );

    // A turn that ended asks for the history and for the uncommitted changes.
    let (history_asked, uncommitted_asked) = (
        stage.keeper.history_asked(),
        stage.keeper.uncommitted_asked(),
    );
    stage.keeper.tell(Fact::TurnEnded {
        workspace: id('1'),
        session: session(),
        turn: TurnId::new("b".repeat(32)),
        // No state was recorded, so the turn selects nothing: what a turn end with a state
        // selects is the test of the row about it.
        state: None,
        since: None,
    });
    wait_until(
        &mut cx,
        "the history and the uncommitted changes asked for",
        |_| {
            stage.keeper.history_asked() > history_asked
                && stage.keeper.uncommitted_asked() > uncommitted_asked
        },
    );

    // The turn leaves the history: the selection of it is gone, and the comparison is
    // back in its place.
    let without = History {
        entries: vec![HistoryEntry::Commit(commit('1', "First"))],
        ..history()
    };
    stage.keeper.set_history(without.clone());
    stage
        .keeper
        .tell(Fact::Workspaces(vec![repository(vec![workspace(
            '1', "feature",
        )])]));
    wait_until(&mut cx, "the shorter history", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.history() == Some(&without))
    });
    assert_eq!(shown(&stage.pane, &mut cx), ChangeShown::Comparison);
    assert_eq!(paths(&stage.pane, &mut cx), ["story.txt"]);
}

#[gpui_kit::test]
fn with_nothing_committed_the_uncommitted_files_are_read_from_nothing(cx: &mut TestAppContext) {
    let nothing = History {
        start: None,
        entries: Vec::new(),
        ..history()
    };
    let (mut stage, mut cx) = open_with_history(nothing, uncommitted(), cx);
    assert!(
        entry_bounds(None, &mut cx).is_some(),
        "the uncommitted entry is drawn alone"
    );
    click_entry(None, Modifiers::none(), &mut cx);
    read_versions(
        &mut stage,
        versions_of(None, "mine.txt", None, "mine.txt"),
        "",
        "one\n",
        &mut cx,
    );
    read_versions(
        &mut stage,
        versions_of(None, "outside.txt", None, "outside.txt"),
        "",
        "two\n",
        &mut cx,
    );
    wait_for_paths(&stage, &["mine.txt", "outside.txt"], &mut cx);
}

#[gpui_kit::test]
fn a_change_with_nothing_before_it_says_so_and_asks_for_no_comparison(cx: &mut TestAppContext) {
    let first_turn = History {
        start: None,
        entries: vec![turn('a', Some(s1()))],
        ..history()
    };
    let (mut stage, mut cx) = open_with_history(first_turn, Vec::new(), cx);
    click_entry(Some(0), Modifiers::none(), &mut cx);
    assert_eq!(
        shown(&stage.pane, &mut cx),
        ChangeShown::Entries { first: 0, last: 0 }
    );
    assert!(stage.pane.read_with(&cx, |pane, _| pane.notice()).is_some());
    // Nothing was asked: what the Window asks next is the changes of another workspace.
    stage
        .keeper
        .tell(Fact::Workspaces(vec![repository(vec![workspace(
            '2', "other",
        )])]));
    let next = stage
        .keeper
        .expect(Request::GetChanges { workspace: id('2') }, &mut cx);
    stage.keeper.reply(
        next,
        Ok(Reply::Changes(yaar_link::Changes {
            workspace: id('2'),
            ..since_main(Vec::new(), Vec::new())
        })),
    );
}

#[gpui_kit::test]
fn the_answer_to_a_selection_that_another_one_has_replaced_is_not_shown(cx: &mut TestAppContext) {
    let (mut stage, mut cx) = open_with_history(history(), uncommitted(), cx);
    click_entry(Some(0), Modifiers::none(), &mut cx);
    let first = stage.keeper.request(&mut cx);
    click_entry(Some(2), Modifiers::none(), &mut cx);
    let second = stage.keeper.request(&mut cx);
    assert_eq!(
        first.1,
        Request::CompareVersions {
            workspace: id('1'),
            from: start(),
            to: Some(c1())
        }
    );
    assert_eq!(
        second.1,
        Request::CompareVersions {
            workspace: id('1'),
            from: s1(),
            to: Some(c2())
        }
    );
    // The Keeper answers the second first, then the one that was replaced.
    stage.keeper.reply(
        second.0,
        Ok(Reply::Compared(vec![file("second.txt", FileStatus::Added)])),
    );
    read_versions(
        &mut stage,
        versions_of(Some(s1()), "second.txt", Some(c2()), "second.txt"),
        "",
        "2\n",
        &mut cx,
    );
    wait_for_paths(&stage, &["second.txt"], &mut cx);
    stage.keeper.reply(
        first.0,
        Ok(Reply::Compared(vec![file("first.txt", FileStatus::Added)])),
    );
    cx.run_until_parked();
    assert_eq!(paths(&stage.pane, &mut cx), ["second.txt"]);
    assert_eq!(
        shown(&stage.pane, &mut cx),
        ChangeShown::Entries { first: 2, last: 2 }
    );
}

#[gpui_kit::test]
fn a_comment_on_the_files_of_a_selection_is_anchored_in_the_versions_the_change_is_between(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with_history(history(), uncommitted(), cx);
    select_the_turn(&mut stage, "before\n", "after\n", &mut cx);
    // The rows are the heading, then the removed line and the added one.
    let anchor_of = |stage: &Stage, cx: &mut VisualTestContext| {
        stage
            .pane
            .read_with(cx, |pane, _| {
                pane.comment_box().and_then(|opened| opened.anchor)
            })
            .expect("a box with an anchor")
    };
    let removed = common::bounds_of_row(1, &mut cx).center();
    cx.simulate_click(removed, Modifiers::none());
    cx.simulate_keystrokes("c");
    let old = anchor_of(&stage, &mut cx);
    assert_eq!(
        (old.side, old.tree.as_deref()),
        (yaar_link::Side::Old, Some(c1().as_str()))
    );
    cx.simulate_keystrokes("escape");
    let added = common::bounds_of_row(2, &mut cx).center();
    cx.simulate_click(added, Modifiers::none());
    cx.simulate_keystrokes("c");
    let new = anchor_of(&stage, &mut cx);
    assert_eq!(
        (new.side, new.tree.as_deref()),
        (yaar_link::Side::New, Some(s1().as_str()))
    );
    assert_eq!(new.text, "after");
    cx.simulate_keystrokes("escape");
}

#[gpui_kit::test]
fn the_tree_is_not_drawn_where_there_is_nothing_in_it_to_select(cx: &mut TestAppContext) {
    let (stage, mut cx) = open_with_history(
        History {
            entries: Vec::new(),
            ..history()
        },
        Vec::new(),
        cx,
    );
    assert_eq!(entry_bounds(None, &mut cx), None);
    assert_eq!(shown(&stage.pane, &mut cx), ChangeShown::Comparison);
}

/// A window that has the history of the tests from its first moment and no answer yet for
/// its changes. The request for them is the first the Keeper hears; its id is given back,
/// to be answered when the test wants the changes to come.
fn open_before_the_changes(cx: &mut TestAppContext) -> (Stage, VisualTestContext, RequestId) {
    open_before_the_changes_of(history(), cx)
}

/// As `open_before_the_changes`, for a history of the test's own.
fn open_before_the_changes_of(
    history: History,
    cx: &mut TestAppContext,
) -> (Stage, VisualTestContext, RequestId) {
    let (models, keeper) = link_up(vec![workspace('1', "feature")], cx);
    keeper.set_history(history);
    keeper.set_uncommitted(uncommitted());
    let (mut stage, mut cx) = open_window(models, keeper, cx);
    let changes = stage
        .keeper
        .expect(Request::GetChanges { workspace: id('1') }, &mut cx);
    (stage, cx, changes)
}

/// Answers the question for the changes of the workspace, then for their one file.
fn answer_the_changes(stage: &mut Stage, asked: RequestId) -> yaar_link::Changes {
    let changes = since_main(
        vec![commit('b', "Edit the story")],
        vec![file("story.txt", FileStatus::Modified)],
    );
    stage
        .keeper
        .reply(asked, Ok(Reply::Changes(changes.clone())));
    changes
}

#[gpui_kit::test]
fn until_the_changes_come_the_last_recorded_turn_is_shown_and_then_the_comparison(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx, changes_asked) = open_before_the_changes(cx);
    // Nothing of git has answered, and the last turn of the history is on screen, read
    // from the states the Keeper keeps.
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
    assert!(matches!(
        rows(&stage.pane, &mut cx).first(),
        Some(PaneRow::File { .. })
    ));

    // The comparison answers, and takes the turn's place.
    let changes = answer_the_changes(&mut stage, changes_asked);
    read_versions(
        &mut stage,
        read_file(&changes, &changes.files[0]),
        "a\nb\n",
        "a\nc\n",
        &mut cx,
    );
    wait_for_paths(&stage, &["story.txt"], &mut cx);
    assert_eq!(shown(&stage.pane, &mut cx), ChangeShown::Comparison);
    assert!(matches!(
        rows(&stage.pane, &mut cx).first(),
        Some(PaneRow::Comparison { .. })
    ));
}

#[gpui_kit::test]
fn the_picture_before_the_changes_come_is_the_newest_turn_that_has_a_state(
    cx: &mut TestAppContext,
) {
    // Two turns with a state, and a turn after them whose state was not recorded: the
    // picture is the change of the second turn, from the commit before it.
    let (mut stage, mut cx, _changes_asked) = open_before_the_changes_of(
        History {
            workspace: id('1'),
            start: Some(start()),
            entries: vec![
                HistoryEntry::Commit(commit('1', "First")),
                turn('a', Some(s1())),
                HistoryEntry::Commit(commit('2', "Second")),
                turn('b', Some(s2())),
                turn('c', None),
            ],
        },
        cx,
    );
    compared(
        &mut stage,
        c2(),
        Some(s2()),
        vec![file("newest.txt", FileStatus::Modified)],
        &mut cx,
    );
    read_versions(
        &mut stage,
        versions_of(Some(c2()), "newest.txt", Some(s2()), "newest.txt"),
        "before\n",
        "after\n",
        &mut cx,
    );
    wait_for_paths(&stage, &["newest.txt"], &mut cx);
    assert_eq!(
        shown(&stage.pane, &mut cx),
        ChangeShown::Entries { first: 3, last: 3 }
    );
}

#[gpui_kit::test]
fn what_the_reviewer_selects_before_the_changes_come_is_not_taken_by_them(cx: &mut TestAppContext) {
    let (mut stage, mut cx, changes_asked) = open_before_the_changes(cx);
    compared(&mut stage, c1(), Some(s1()), Vec::new(), &mut cx);
    wait_until(&mut cx, "the last turn shown", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.change_shown() != ChangeShown::Comparison)
    });
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
        "2\n",
        &mut cx,
    );
    wait_for_paths(&stage, &["second.txt"], &mut cx);

    answer_the_changes(&mut stage, changes_asked);
    wait_until(&mut cx, "the changes answered", |cx| {
        stage
            .models
            .changes
            .read_with(cx, |changes, _| changes.changes().is_some())
    });
    cx.run_until_parked();
    assert_eq!(
        shown(&stage.pane, &mut cx),
        ChangeShown::Entries { first: 2, last: 2 }
    );
    assert_eq!(paths(&stage.pane, &mut cx), ["second.txt"]);
    // No file of the comparison was asked for: another workspace's changes are the next
    // request the Window makes.
    stage
        .keeper
        .tell(Fact::Workspaces(vec![repository(vec![workspace(
            '2', "other",
        )])]));
    let next = stage
        .keeper
        .expect(Request::GetChanges { workspace: id('2') }, &mut cx);
    stage.keeper.reply(
        next,
        Ok(Reply::Changes(yaar_link::Changes {
            workspace: id('2'),
            ..since_main(Vec::new(), Vec::new())
        })),
    );
}

#[gpui_kit::test]
fn the_rows_take_the_focus_back_when_the_tree_it_was_in_is_gone(cx: &mut TestAppContext) {
    let (mut stage, mut cx) = open_with_history(history(), Vec::new(), cx);
    click_entry(Some(0), Modifiers::none(), &mut cx);
    compared(&mut stage, start(), Some(c1()), Vec::new(), &mut cx);
    cx.update(|window, _| assert!(!stage.focus.is_focused(window), "the tree has the focus"));

    // Every entry leaves the history, and nothing is uncommitted: there is no tree.
    let empty = History {
        entries: Vec::new(),
        ..history()
    };
    stage.keeper.set_history(empty.clone());
    stage
        .keeper
        .tell(Fact::Workspaces(vec![repository(vec![workspace(
            '1', "feature",
        )])]));
    wait_until(&mut cx, "the empty history", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.history() == Some(&empty))
    });
    assert_eq!(entry_bounds(Some(0), &mut cx), None);
    assert_eq!(shown(&stage.pane, &mut cx), ChangeShown::Comparison);
    // The rows take the focus when the frame without the tree is drawn.
    cx.run_until_parked();
    cx.update(|window, _| assert!(stage.focus.is_focused(window), "the rows have the focus"));
}

/// The state recorded at the end of the turns that end in the tests below.
fn s2() -> String {
    version('6')
}

fn s3() -> String {
    version('7')
}

/// The review of the session of the history, which the reviewer last marked reviewed at
/// the state of the turn T1.
fn review_of_the_session() -> ReviewId {
    ReviewId::new("7".repeat(32))
}

/// A window whose Keeper holds that review of the session alone, shown by the pane.
fn open_with_the_review_of_the_session(cx: &mut TestAppContext) -> (Stage, VisualTestContext) {
    let review = review_with(review_of_the_session(), Some(session()), Some(s1()));
    open_with_reviews(
        history(),
        uncommitted(),
        Some(reviews_of(vec![review])),
        review_of_the_session(),
        cx,
    )
}

/// What the Keeper tells when a turn of the session ends in workspace `1`.
fn turn_ended(state: Option<String>, since: Option<String>) -> Fact {
    Fact::TurnEnded {
        workspace: id('1'),
        session: session(),
        turn: TurnId::new("c".repeat(32)),
        state,
        since,
    }
}

/// Keeps the app running until the element with this selector is drawn.
fn wait_drawn(selector: &str, cx: &mut VisualTestContext) {
    let deadline = std::time::Instant::now() + common::DEADLINE;
    while drawn(selector, cx).is_none() {
        cx.run_until_parked();
        assert!(
            std::time::Instant::now() < deadline,
            "waited in vain for {selector} to be drawn"
        );
        std::thread::park_timeout(std::time::Duration::from_millis(1));
    }
}

/// Answers the comparison a turn's end asks for, which the review of the turn's session
/// makes: the files of the change, each credited to that session.
fn compared_for_the_session(
    stage: &mut Stage,
    from: String,
    to: String,
    files: Vec<ChangedFile>,
    cx: &mut VisualTestContext,
) {
    let asked = stage.keeper.expect(
        Request::CompareForReview {
            workspace: id('1'),
            review: review_of_the_session(),
            from,
            to,
        },
        cx,
    );
    let credited = files
        .into_iter()
        .map(|file| CreditedFile {
            file,
            by: vec![session()],
        })
        .collect();
    stage
        .keeper
        .reply(asked, Ok(Reply::ComparedForReview(credited)));
}

/// Answers the comparison of a turn's change that the reviewer selected in the tree, and
/// the read of its one file.
fn show_the_turn(
    stage: &mut Stage,
    from: String,
    to: String,
    path: &str,
    cx: &mut VisualTestContext,
) {
    compared(
        stage,
        from.clone(),
        Some(to.clone()),
        vec![file(path, FileStatus::Added)],
        cx,
    );
    read_the_turn(stage, from, to, path, cx);
}

/// Answers the comparison a turn's end, or the offer it made, asks for, and the read of its
/// one file.
fn show_the_end_of_the_turn(
    stage: &mut Stage,
    from: String,
    to: String,
    path: &str,
    cx: &mut VisualTestContext,
) {
    compared_for_the_session(
        stage,
        from.clone(),
        to.clone(),
        vec![file(path, FileStatus::Added)],
        cx,
    );
    read_the_turn(stage, from, to, path, cx);
}

/// Answers the read of the one file of a turn's change.
fn read_the_turn(
    stage: &mut Stage,
    from: String,
    to: String,
    path: &str,
    cx: &mut VisualTestContext,
) {
    read_versions(
        stage,
        versions_of(Some(from), path, Some(to), path),
        "",
        "x\n",
        cx,
    );
    wait_for_paths(stage, &[path], cx);
}

/// Proves that nothing else was asked since the last request the test read: a click on
/// the first commit is the next request the Window sends.
fn nothing_else_was_asked(stage: &mut Stage, cx: &mut VisualTestContext) {
    click_entry(Some(0), Modifiers::none(), cx);
    compared(stage, start(), Some(c1()), Vec::new(), cx);
    wait_until(cx, "the first commit shown", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.change_shown() == ChangeShown::Entries { first: 0, last: 0 }
        })
    });
}

#[gpui_kit::test]
fn when_a_turn_ends_changes_selects_what_changed_since_the_last_review(cx: &mut TestAppContext) {
    let (mut stage, mut cx) = open_with_the_review_of_the_session(cx);
    assert_eq!(shown(&stage.pane, &mut cx), ChangeShown::Comparison);

    // The review was marked reviewed at S1; the turn ended at S2.
    stage.keeper.tell(turn_ended(Some(s2()), Some(s1())));
    wait_until(&mut cx, "the change since the last review", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.change_shown()
                == ChangeShown::Since {
                    from: Some(s1()),
                    to: s2(),
                }
        })
    });
    show_the_end_of_the_turn(&mut stage, s1(), s2(), "turn.txt", &mut cx);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.offer().cloned()),
        None,
        "nothing is offered where the selection is taken"
    );
    assert!(drawn("review-offer", &mut cx).is_none());

    // Before the first review there is no state it counts from: the history's start.
    stage.keeper.tell(turn_ended(Some(s3()), None));
    wait_until(&mut cx, "the change since the start", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.change_shown()
                == ChangeShown::Since {
                    from: Some(start()),
                    to: s3(),
                }
        })
    });
    show_the_end_of_the_turn(&mut stage, start(), s3(), "later.txt", &mut cx);

    // A turn whose state was not recorded changes no selection: the Keeper's answers for
    // the history and the uncommitted changes, which that turn end makes the Window ask
    // for, are the barrier, and the next request is a click's.
    let asked = (
        stage.keeper.history_asked(),
        stage.keeper.uncommitted_asked(),
    );
    stage.keeper.tell(turn_ended(None, Some(s1())));
    wait_until(
        &mut cx,
        "the history and the changes asked for again",
        |_| stage.keeper.history_asked() > asked.0 && stage.keeper.uncommitted_asked() > asked.1,
    );
    assert_eq!(
        shown(&stage.pane, &mut cx),
        ChangeShown::Since {
            from: Some(start()),
            to: s3()
        }
    );
    nothing_else_was_asked(&mut stage, &mut cx);
}

#[gpui_kit::test]
fn a_turn_that_ends_in_a_workspace_not_on_screen_selects_nothing(cx: &mut TestAppContext) {
    let (mut stage, mut cx) = open_with_the_review_of_the_session(cx);
    stage.keeper.tell(Fact::TurnEnded {
        workspace: id('2'),
        session: session(),
        turn: TurnId::new("c".repeat(32)),
        state: Some(s2()),
        since: Some(s1()),
    });
    // The barrier: a turn end of the workspace on screen, which asks for its history.
    let asked = stage.keeper.history_asked();
    stage.keeper.tell(turn_ended(None, None));
    wait_until(&mut cx, "the history asked for again", |_| {
        stage.keeper.history_asked() > asked
    });
    assert_eq!(shown(&stage.pane, &mut cx), ChangeShown::Comparison);
    nothing_else_was_asked(&mut stage, &mut cx);
}

#[gpui_kit::test]
fn set_to_keep_the_selection_changes_stays_as_it_is_at_a_turn_end_and_offers_what_changed_since_the_last_review(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with_the_review_of_the_session(cx);
    focus_the_rows(&stage, &mut cx);
    let keeps = |stage: &Stage, cx: &mut VisualTestContext| {
        stage.pane.read_with(cx, |pane, _| pane.keeps_selection())
    };
    assert!(
        !keeps(&stage, &mut cx),
        "the setting is off until it is set"
    );
    cx.simulate_keystrokes("shift-t");
    assert!(keeps(&stage, &mut cx));

    // T1 is selected.
    click_entry(Some(1), Modifiers::none(), &mut cx);
    show_the_turn(&mut stage, c1(), s1(), "turn.txt", &mut cx);
    assert_eq!(
        shown(&stage.pane, &mut cx),
        ChangeShown::Entries { first: 1, last: 1 }
    );

    // A turn ends: nothing changes but the offer, which is drawn before anything else is
    // done.
    stage.keeper.tell(turn_ended(Some(s2()), Some(s1())));
    wait_drawn("review-offer", &mut cx);
    let since = ChangeShown::Since {
        from: Some(s1()),
        to: s2(),
    };
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.offer().cloned()),
        Some(since.clone())
    );
    assert_eq!(
        shown(&stage.pane, &mut cx),
        ChangeShown::Entries { first: 1, last: 1 }
    );
    assert_eq!(paths(&stage.pane, &mut cx), ["turn.txt"]);

    // The next request of the Window is that of the click that follows: the offer asked
    // for nothing.
    click_entry(Some(0), Modifiers::none(), &mut cx);
    compared(&mut stage, start(), Some(c1()), Vec::new(), &mut cx);
    wait_until(&mut cx, "the first commit shown", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.change_shown() == ChangeShown::Entries { first: 0, last: 0 }
        })
    });
    // Selecting takes the offer away.
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.offer().cloned()),
        None
    );
    assert!(drawn("review-offer", &mut cx).is_none());

    // The offer made again by a second turn end is taken by its key.
    stage.keeper.tell(turn_ended(Some(s3()), Some(s2())));
    wait_drawn("review-offer", &mut cx);
    focus_the_rows(&stage, &mut cx);
    cx.simulate_keystrokes("shift-o");
    show_the_end_of_the_turn(&mut stage, s2(), s3(), "second.txt", &mut cx);
    assert_eq!(
        shown(&stage.pane, &mut cx),
        ChangeShown::Since {
            from: Some(s2()),
            to: s3()
        }
    );
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.offer().cloned()),
        None
    );

    // And by a click on its control.
    stage.keeper.tell(turn_ended(Some(s2()), Some(s1())));
    wait_drawn("review-offer-take", &mut cx);
    let at = drawn("review-offer-take", &mut cx)
        .expect("the control is drawn")
        .center();
    cx.simulate_click(at, Modifiers::none());
    show_the_end_of_the_turn(&mut stage, s1(), s2(), "third.txt", &mut cx);
    assert_eq!(shown(&stage.pane, &mut cx), since);
    assert!(drawn("review-offer", &mut cx).is_none());

    // The key sets the pane back to selecting.
    focus_the_rows(&stage, &mut cx);
    cx.simulate_keystrokes("shift-t");
    assert!(!keeps(&stage, &mut cx));
}

#[gpui_kit::test]
fn the_offer_is_not_made_for_a_turn_whose_state_was_not_recorded(cx: &mut TestAppContext) {
    let (mut stage, mut cx) = open_with_the_review_of_the_session(cx);
    focus_the_rows(&stage, &mut cx);
    cx.simulate_keystrokes("shift-t");
    let asked = stage.keeper.history_asked();
    stage.keeper.tell(turn_ended(None, Some(s1())));
    wait_until(&mut cx, "the history asked for again", |_| {
        stage.keeper.history_asked() > asked
    });
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.offer().cloned()),
        None
    );
    assert!(drawn("review-offer", &mut cx).is_none());
    nothing_else_was_asked(&mut stage, &mut cx);
}

#[gpui_kit::test]
fn the_marks_of_the_reviewer_stay_when_the_history_is_read_again_under_a_selection(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with_history(history(), uncommitted(), cx);
    click_entry(Some(1), Modifiers::none(), &mut cx);
    show_the_turn(&mut stage, c1(), s1(), "turn.txt", &mut cx);
    // The cursor is on the turn: up to the first commit, and a mark on it, not shown yet.
    cx.simulate_keystrokes("k space");
    let marked = |stage: &Stage, cx: &mut VisualTestContext| {
        stage.pane.read_with(cx, |pane, _| pane.marked_changes())
    };
    assert_eq!(marked(&stage, &mut cx), [0]);

    // A commit joins the history, which the pane reads again: the entries it shows are as
    // they were, and so are the marks.
    let mut entries = history().entries;
    entries.push(HistoryEntry::Commit(commit('3', "Third")));
    let grown = History {
        entries,
        ..history()
    };
    stage.keeper.set_history(grown.clone());
    stage
        .keeper
        .tell(Fact::Workspaces(vec![repository(vec![workspace(
            '1', "feature",
        )])]));
    wait_until(&mut cx, "the longer history", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.history() == Some(&grown))
    });
    assert_eq!(marked(&stage, &mut cx), [0]);
    assert_eq!(
        shown(&stage.pane, &mut cx),
        ChangeShown::Entries { first: 1, last: 1 }
    );
}

/// Whether the pane keeps the selection at a turn end, as it says.
fn keeps(stage: &Stage, cx: &mut VisualTestContext) -> bool {
    stage.pane.read_with(cx, |pane, _| pane.keeps_selection())
}

fn turn_end_setting(selects: TurnEndSelects) -> Fact {
    Fact::SettingChanged(Setting::TurnEnd(selects))
}

#[gpui_kit::test]
fn what_a_turn_end_selects_follows_the_setting_the_keeper_tells_with_no_key_pressed(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with_the_review_of_the_session(cx);
    assert!(!keeps(&stage, &mut cx), "none is set: the turn end selects");

    // Another Window, or the settings screen, set it: the pane keeps the selection.
    stage
        .keeper
        .tell(turn_end_setting(TurnEndSelects::KeepSelection));
    wait_until(&mut cx, "the pane to keep the selection", |cx| {
        stage.pane.read_with(cx, |pane, _| pane.keeps_selection())
    });
    wait_drawn("review-keeps-selection", &mut cx);
    stage.keeper.tell(turn_ended(Some(s2()), Some(s1())));
    wait_drawn("review-offer", &mut cx);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.offer().cloned()),
        Some(ChangeShown::Since {
            from: Some(s1()),
            to: s2()
        })
    );
    assert_eq!(shown(&stage.pane, &mut cx), ChangeShown::Comparison);
    // Nothing was selected, so nothing was asked for: the next request is a click's.
    nothing_else_was_asked(&mut stage, &mut cx);

    // Set back, the next turn end selects what changed.
    stage
        .keeper
        .tell(turn_end_setting(TurnEndSelects::WhatChanged));
    wait_until(&mut cx, "the pane to select at a turn end", |cx| {
        !stage.pane.read_with(cx, |pane, _| pane.keeps_selection())
    });
    assert!(drawn("review-keeps-selection", &mut cx).is_none());
    stage.keeper.tell(turn_ended(Some(s3()), Some(s2())));
    wait_until(&mut cx, "the change since the last review", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.change_shown()
                == ChangeShown::Since {
                    from: Some(s2()),
                    to: s3(),
                }
        })
    });
    show_the_end_of_the_turn(&mut stage, s2(), s3(), "later.txt", &mut cx);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.offer().cloned()),
        None
    );
}

#[gpui_kit::test]
fn a_pane_opened_where_the_setting_is_already_set_keeps_the_selection_before_any_key_or_fact(
    cx: &mut TestAppContext,
) {
    let (models, keeper) = link_up_staged(
        vec![workspace('1', "feature")],
        |keeper| keeper.set_settings(vec![Setting::TurnEnd(TurnEndSelects::KeepSelection)]),
        cx,
    );
    wait_until(cx, "the settings in the models", |cx| {
        models
            .settings
            .read_with(cx, |settings, _| settings.turn_end())
            == TurnEndSelects::KeepSelection
    });
    let (stage, mut cx) = open_window(models, keeper, cx);
    assert!(keeps(&stage, &mut cx));
    cx.run_until_parked();
    assert!(drawn("review-keeps-selection", &mut cx).is_some());
}

#[gpui_kit::test]
fn the_key_that_keeps_the_selection_sets_the_setting_and_flips_what_the_setting_says_now(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with_the_review_of_the_session(cx);
    focus_the_rows(&stage, &mut cx);
    let turn_end = |stage: &Stage, cx: &mut VisualTestContext| {
        stage
            .models
            .settings
            .read_with(cx, |settings, _| settings.turn_end())
    };

    cx.simulate_keystrokes("shift-t");
    // At once in the models, and the Keeper keeps it.
    assert_eq!(turn_end(&stage, &mut cx), TurnEndSelects::KeepSelection);
    assert!(keeps(&stage, &mut cx));
    wait_until(&mut cx, "the setting to reach the Keeper", |_| {
        stage.keeper.settings_set() == [Setting::TurnEnd(TurnEndSelects::KeepSelection)]
    });

    // A change told from outside is what the key flips, not what the pane did last.
    stage
        .keeper
        .tell(turn_end_setting(TurnEndSelects::WhatChanged));
    wait_until(&mut cx, "the pane to follow the setting", |cx| {
        !stage.pane.read_with(cx, |pane, _| pane.keeps_selection())
    });
    cx.simulate_keystrokes("shift-t");
    assert!(keeps(&stage, &mut cx));
    cx.simulate_keystrokes("shift-t");
    assert!(!keeps(&stage, &mut cx));
    assert_eq!(turn_end(&stage, &mut cx), TurnEndSelects::WhatChanged);
    wait_until(&mut cx, "the three sets to reach the Keeper", |_| {
        stage.keeper.settings_set()
            == [
                Setting::TurnEnd(TurnEndSelects::KeepSelection),
                Setting::TurnEnd(TurnEndSelects::KeepSelection),
                Setting::TurnEnd(TurnEndSelects::WhatChanged),
            ]
    });
}

/// The history of the tests with the turn T1 expired, between the two commits, and a turn
/// after the second commit whose state is still there.
fn history_with_an_expired_turn() -> History {
    History {
        workspace: id('1'),
        start: Some(start()),
        entries: vec![
            HistoryEntry::Commit(commit('1', "First")),
            HistoryEntry::ExpiredTurn {
                turn: TurnId::new("a".repeat(32)),
                session: session(),
            },
            HistoryEntry::Commit(commit('2', "Second")),
            turn('b', Some(s2())),
        ],
    }
}

#[gpui_kit::test]
fn a_turn_whose_recording_expired_is_drawn_with_its_words_and_cannot_be_selected(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with_history(history_with_an_expired_turn(), uncommitted(), cx);
    assert_eq!(EXPIRED_TURN, "a turn (its recording has expired)");
    // The words of a row are in the selector of its text.
    let says = |index: &str, words: &str, cx: &mut VisualTestContext| {
        drawn(&format!("review-history-{index}:{words}"), cx).is_some()
    };
    assert!(says("1", &format!("An agent, {EXPIRED_TURN}"), &mut cx));
    assert!(says("3", "An agent, a turn", &mut cx));
    assert!(says("uncommitted", "Uncommitted changes (2)", &mut cx));
    assert!(!says("1", "An agent, a turn", &mut cx));
    assert!(entry_bounds(Some(1), &mut cx).is_some(), "the row is drawn");

    // A click on it selects nothing, and neither does Enter on the cursor that the click
    // left on it.
    click_entry(Some(1), Modifiers::none(), &mut cx);
    assert_eq!(shown(&stage.pane, &mut cx), ChangeShown::Comparison);
    focus_the_rows(&stage, &mut cx);
    cx.simulate_keystrokes("tab");
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.history_cursor()),
        Some(1)
    );
    cx.simulate_keystrokes("enter");
    assert_eq!(shown(&stage.pane, &mut cx), ChangeShown::Comparison);
    // It cannot be marked for a range either: Enter with nothing marked shows nothing.
    cx.simulate_keystrokes("space");
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.marked_changes()),
        Vec::<usize>::new()
    );
    cx.simulate_keystrokes("enter");
    assert_eq!(shown(&stage.pane, &mut cx), ChangeShown::Comparison);
    // Nothing was asked for either: the next request is the click on the first commit's.
    nothing_else_was_asked(&mut stage, &mut cx);

    // A turn below it is selected as before, from the commit above it.
    click_entry(Some(3), Modifiers::none(), &mut cx);
    show_the_turn(&mut stage, c2(), s2(), "later.txt", &mut cx);
    // The commit just below it counts from the nearest version above it, which is the commit
    // before the expired turn: no recording of the turn is looked for.
    click_entry(Some(2), Modifiers::none(), &mut cx);
    compared(&mut stage, c1(), Some(c2()), Vec::new(), &mut cx);
    wait_until(&mut cx, "the second commit shown", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.change_shown() == ChangeShown::Entries { first: 2, last: 2 }
        })
    });
}

#[gpui_kit::test]
fn the_history_is_asked_again_when_the_states_of_the_workspace_on_screen_were_deleted(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with_history(history(), uncommitted(), cx);
    let asked = stage.keeper.history_asked();
    let expired = History {
        entries: vec![
            HistoryEntry::Commit(commit('1', "First")),
            HistoryEntry::ExpiredTurn {
                turn: TurnId::new("a".repeat(32)),
                session: session(),
            },
            HistoryEntry::Commit(commit('2', "Second")),
        ],
        ..history()
    };
    stage.keeper.set_history(expired.clone());
    // Another workspace's states went: this pane does not ask. Nothing shows a question
    // that is not asked, so a fact of another kind that makes a question follows it: the
    // Keeper answers the questions of one Window in order, so when that question is heard
    // any question the first fact made was heard before it.
    let uncommitted = stage.keeper.uncommitted_asked();
    stage
        .keeper
        .tell(Fact::StatesDeleted { workspace: id('2') });
    stage
        .keeper
        .tell(Fact::ChangesChanged { workspace: id('1') });
    wait_until(&mut cx, "the barrier's question to be heard", |_| {
        stage.keeper.uncommitted_asked() > uncommitted
    });
    assert_eq!(
        stage.keeper.history_asked(),
        asked,
        "the states of another workspace went: no history was asked for"
    );
    // The states of the workspace on screen went: the history is asked for again, and the
    // turn that expired shows.
    stage
        .keeper
        .tell(Fact::StatesDeleted { workspace: id('1') });
    wait_until(&mut cx, "the history with the expired turn", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.history() == Some(&expired))
    });
    assert_eq!(stage.keeper.history_asked(), asked + 1);
}

//! Component test: the review pane over a change two sessions made. The test plays the Keeper on a
//! socket pair.

mod common;

use std::path::PathBuf;

use gpui_kit::{Entity, Modifiers, TestAppContext, VisualTestContext};
use yaar_link::{
    CreditedFile, Fact, FileStatus, History, HistoryEntry, Refusal, Reply, Request, ReviewId,
    SessionId, SessionState, TurnId,
};
use yaar_review_pane::{ChangeShown, PaneRow, ReviewPane};

use common::{
    COMMIT, Stage, agents_of, commit, drawn, file, id, link_up, open_window, repository,
    reviews_of, rows, session, since_main, texts, wait_for_files, wait_until, workspace,
};

fn version(digit: char) -> String {
    digit.to_string().repeat(40)
}

fn start() -> String {
    COMMIT.to_owned()
}

fn c1() -> String {
    version('1')
}

fn s1() -> String {
    version('5')
}

fn c2() -> String {
    version('2')
}

fn s2() -> String {
    version('6')
}

fn s3() -> String {
    version('7')
}

fn session_s() -> SessionId {
    SessionId::new("5".repeat(32))
}

fn session_t() -> SessionId {
    SessionId::new("6".repeat(32))
}

fn review_s() -> ReviewId {
    ReviewId::new("a".repeat(32))
}

fn review_t() -> ReviewId {
    ReviewId::new("b".repeat(32))
}

fn review_of_no_agent() -> ReviewId {
    ReviewId::new("c".repeat(32))
}

fn history() -> History {
    History {
        workspace: id('1'),
        start: Some(start()),
        entries: vec![
            HistoryEntry::Commit(commit('1', "First")),
            HistoryEntry::Turn {
                turn: TurnId::new("a".repeat(32)),
                session: session_s(),
                state: Some(s1()),
            },
            HistoryEntry::Commit(commit('2', "Second")),
        ],
    }
}

fn credited(path: &str, by: Vec<SessionId>) -> CreditedFile {
    CreditedFile {
        file: file(path, FileStatus::Modified),
        by,
    }
}

fn the_list_of_s() -> Vec<CreditedFile> {
    vec![
        credited("b.txt", vec![session_s()]),
        credited("h.txt", Vec::new()),
        credited("s.txt", vec![session_s(), session_t()]),
    ]
}

/// The workspace's changes are answered first: a pane without them shows the last recorded turn.
fn open_with(
    reviews: Vec<(ReviewId, Option<SessionId>)>,
    shown: &ReviewId,
    uncommitted: Vec<CreditedFile>,
    cx: &mut TestAppContext,
) -> (Stage, VisualTestContext) {
    let (models, keeper) = link_up(vec![workspace('1', "feature")], cx);
    keeper.set_uncommitted(uncommitted);
    keeper.set_reviews(reviews_of(
        reviews
            .into_iter()
            .map(|(review, agent)| common::review_with(review, agent, Some(s1())))
            .collect(),
    ));
    let (mut stage, mut cx) = open_window(models, keeper, cx);
    let changes = since_main(Vec::new(), vec![file("story.txt", FileStatus::Modified)]);
    stage
        .keeper
        .show(&changes, |_| texts("a\n", "b\n"), &mut cx);
    wait_for_files(&stage.pane, &mut cx);
    let asked = stage.keeper.history_asked();
    stage.keeper.set_history(history());
    stage
        .keeper
        .tell(Fact::Workspaces(vec![repository(vec![workspace(
            '1', "feature",
        )])]));
    wait_until(&mut cx, "the history in the pane", |cx| {
        stage.keeper.history_asked() > asked
            && stage
                .pane
                .read_with(cx, |pane, _| pane.history() == Some(&history()))
    });
    wait_until(&mut cx, "the uncommitted changes of the review", |cx| {
        stage.keeper.uncommitted_review().as_ref() == Some(shown)
            && stage
                .pane
                .read_with(cx, |pane, _| pane.uncommitted().is_some())
    });
    (stage, cx)
}

fn turn_ended(session: &SessionId, state: &str, since: &str) -> Fact {
    Fact::TurnEnded {
        workspace: id('1'),
        session: session.clone(),
        turn: TurnId::new("c".repeat(32)),
        state: Some(state.to_owned()),
        since: Some(since.to_owned()),
    }
}

fn compare_for(review: &ReviewId, from: String, to: String) -> Request {
    Request::CompareForReview {
        workspace: id('1'),
        review: review.clone(),
        from,
        to,
    }
}

fn listed_for(
    stage: &mut Stage,
    review: &ReviewId,
    from: &str,
    to: &str,
    files: Vec<CreditedFile>,
    cx: &mut VisualTestContext,
) {
    let asked = stage
        .keeper
        .expect(compare_for(review, from.to_owned(), to.to_owned()), cx);
    stage
        .keeper
        .reply(asked, Ok(Reply::ComparedForReview(files)));
}

fn read_files(
    stage: &mut Stage,
    old: Option<&str>,
    new: Option<&str>,
    files: &[(&str, &str, &str)],
    cx: &mut VisualTestContext,
) {
    for (path, before, after) in files {
        let asked = stage.keeper.expect(
            Request::ReadVersions {
                workspace: id('1'),
                old: old.map(str::to_owned),
                old_path: PathBuf::from(path),
                new: new.map(str::to_owned),
                path: PathBuf::from(path),
            },
            cx,
        );
        stage
            .keeper
            .reply(asked, texts(before, after).map(Reply::File));
    }
    let wanted: Vec<&str> = files.iter().map(|(path, ..)| *path).collect();
    wait_until(cx, "the files of the change shown", |cx| {
        paths(&stage.pane, cx) == wanted
            && !rows(&stage.pane, cx)
                .iter()
                .any(|row| matches!(row, PaneRow::Unshown { .. }))
    });
}

fn paths(pane: &Entity<ReviewPane>, cx: &mut TestAppContext) -> Vec<String> {
    rows(pane, cx)
        .into_iter()
        .filter_map(|row| match row {
            PaneRow::File { path, .. } => Some(path.display().to_string()),
            _ => None,
        })
        .collect()
}

fn row_of(stage: &Stage, path: &str, cx: &mut VisualTestContext) -> usize {
    rows(&stage.pane, cx)
        .iter()
        .position(|row| matches!(row, PaneRow::File { path: held, .. } if held == path))
        .unwrap_or_else(|| panic!("{path} has no heading among the rows"))
}

/// Whether the pane marks `path` as changed by another agent too; asserts that its model
/// and the drawn heading agree.
fn marked(stage: &Stage, path: &str, cx: &mut VisualTestContext) -> bool {
    let by_the_model = stage.pane.read_with(cx, |pane, _| {
        pane.changed_by_another_too(&PathBuf::from(path))
    });
    let row = row_of(stage, path, cx);
    let drawn_on_it = drawn(&format!("review-shared-{row}"), cx).is_some();
    assert_eq!(
        by_the_model, drawn_on_it,
        "{path}: the model says {by_the_model}, the heading draws {drawn_on_it}"
    );
    by_the_model
}

fn shown(stage: &Stage, cx: &mut VisualTestContext) -> ChangeShown {
    stage.pane.read_with(cx, |pane, _| pane.change_shown())
}

fn click(selector: &str, cx: &mut VisualTestContext) {
    let at = drawn(selector, cx)
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
        .center();
    cx.simulate_click(at, Modifiers::none());
}

fn focus_the_rows(stage: &Stage, cx: &mut VisualTestContext) {
    cx.update(|window, cx| stage.focus.focus(window, cx));
}

/// Proves the Window asked nothing since the last request the test read: the request a click
/// on the first commit sends must be the next one the Keeper hears.
fn nothing_else_was_asked(stage: &mut Stage, cx: &mut VisualTestContext) {
    click("review-history-0", cx);
    let asked = stage.keeper.expect(
        Request::CompareVersions {
            workspace: id('1'),
            from: start(),
            to: Some(c1()),
        },
        cx,
    );
    stage.keeper.reply(asked, Ok(Reply::Compared(Vec::new())));
    wait_until(cx, "the first commit shown", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.change_shown() == ChangeShown::Entries { first: 0, last: 0 }
        })
    });
}

#[gpui_kit::test]
fn a_turn_end_lists_the_files_the_review_of_its_session_shows_and_marks_the_one_another_agent_changed_too(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with(
        vec![(review_s(), Some(session_s()))],
        &review_s(),
        Vec::new(),
        cx,
    );
    stage.keeper.tell(turn_ended(&session_s(), &s2(), &s1()));
    listed_for(
        &mut stage,
        &review_s(),
        &s1(),
        &s2(),
        the_list_of_s(),
        &mut cx,
    );
    read_files(
        &mut stage,
        Some(&s1()),
        Some(&s2()),
        &[
            ("b.txt", "b\n", "b2\n"),
            ("h.txt", "h\n", "h2\n"),
            ("s.txt", "top\n", "top\nfrom s\nfrom t\n"),
        ],
        &mut cx,
    );

    assert_eq!(
        shown(&stage, &mut cx),
        ChangeShown::Since {
            from: Some(s1()),
            to: s2()
        }
    );
    assert!(marked(&stage, "s.txt", &mut cx));
    assert!(!marked(&stage, "b.txt", &mut cx));
    assert!(!marked(&stage, "h.txt", &mut cx));
    let at = row_of(&stage, "s.txt", &mut cx);
    let whole: Vec<String> = rows(&stage.pane, &mut cx)
        .iter()
        .enumerate()
        .filter(|(_, row)| matches!(row, PaneRow::Diff { path, .. } if path == "s.txt"))
        .flat_map(|(index, _)| ["from s", "from t"].map(|line| (index, line)))
        .filter(|(index, line)| {
            drawn(&format!("review-text-{index}-new:{line}"), &mut cx).is_some()
        })
        .map(|(_, line)| line.to_owned())
        .collect();
    assert_eq!(whole, ["from s", "from t"], "rows after heading {at}");
}

#[gpui_kit::test]
fn the_offer_of_a_turn_end_is_listed_through_the_review_of_its_session_when_it_is_taken(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with(
        vec![
            (review_t(), Some(session_t())),
            (review_s(), Some(session_s())),
        ],
        &review_t(),
        Vec::new(),
        cx,
    );
    focus_the_rows(&stage, &mut cx);
    cx.simulate_keystrokes("shift-t");
    assert!(stage.pane.read_with(&cx, |pane, _| pane.keeps_selection()));

    stage.keeper.tell(turn_ended(&session_s(), &s2(), &s1()));
    wait_until(&mut cx, "the offer", |cx| {
        stage.pane.read_with(cx, |pane, _| pane.offer().is_some())
    });
    nothing_else_was_asked(&mut stage, &mut cx);
    assert_eq!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.review_session().cloned()),
        Some(session_t())
    );

    stage.keeper.tell(turn_ended(&session_s(), &s3(), &s2()));
    wait_until(&mut cx, "the offer", |cx| {
        stage.pane.read_with(cx, |pane, _| pane.offer().is_some())
    });
    focus_the_rows(&stage, &mut cx);
    cx.simulate_keystrokes("shift-o");
    listed_for(
        &mut stage,
        &review_s(),
        &s2(),
        &s3(),
        the_list_of_s(),
        &mut cx,
    );
    read_files(
        &mut stage,
        Some(&s2()),
        Some(&s3()),
        &[
            ("b.txt", "b\n", "b2\n"),
            ("h.txt", "h\n", "h2\n"),
            ("s.txt", "s\n", "s2\n"),
        ],
        &mut cx,
    );
    assert_eq!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.review_session().cloned()),
        Some(session_s())
    );
    assert!(marked(&stage, "s.txt", &mut cx));
    assert!(!marked(&stage, "b.txt", &mut cx));
}

#[gpui_kit::test]
fn a_turn_end_of_a_session_the_pane_holds_no_review_of_is_listed_with_no_credits(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with(
        vec![(review_of_no_agent(), None)],
        &review_of_no_agent(),
        Vec::new(),
        cx,
    );
    stage.keeper.tell(turn_ended(&session_s(), &s2(), &s1()));
    let asked = stage.keeper.expect(
        Request::CompareVersions {
            workspace: id('1'),
            from: s1(),
            to: Some(s2()),
        },
        &mut cx,
    );
    stage.keeper.reply(
        asked,
        Ok(Reply::Compared(vec![file("s.txt", FileStatus::Modified)])),
    );
    read_files(
        &mut stage,
        Some(&s1()),
        Some(&s2()),
        &[("s.txt", "s\n", "s2\n")],
        &mut cx,
    );
    assert!(!marked(&stage, "s.txt", &mut cx));
}

#[gpui_kit::test]
fn a_selection_in_the_tree_is_listed_with_no_credits_and_takes_the_marks_of_the_list_before_it_away(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with(
        vec![(review_s(), Some(session_s()))],
        &review_s(),
        Vec::new(),
        cx,
    );
    stage.keeper.tell(turn_ended(&session_s(), &s2(), &s1()));
    listed_for(
        &mut stage,
        &review_s(),
        &s1(),
        &s2(),
        the_list_of_s(),
        &mut cx,
    );
    read_files(
        &mut stage,
        Some(&s1()),
        Some(&s2()),
        &[
            ("b.txt", "b\n", "b2\n"),
            ("h.txt", "h\n", "h2\n"),
            ("s.txt", "s\n", "s2\n"),
        ],
        &mut cx,
    );
    assert!(marked(&stage, "s.txt", &mut cx));
    click("review-history-1", &mut cx);
    let asked = stage.keeper.expect(
        Request::CompareVersions {
            workspace: id('1'),
            from: c1(),
            to: Some(s1()),
        },
        &mut cx,
    );
    stage.keeper.reply(
        asked,
        Ok(Reply::Compared(vec![file("s.txt", FileStatus::Modified)])),
    );
    read_files(
        &mut stage,
        Some(&c1()),
        Some(&s1()),
        &[("s.txt", "s\n", "s2\n")],
        &mut cx,
    );
    assert!(!marked(&stage, "s.txt", &mut cx));
}

#[gpui_kit::test]
fn a_change_listed_with_no_credits_after_one_listed_with_credits_carries_no_mark(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with(
        vec![(review_s(), Some(session_s()))],
        &review_s(),
        Vec::new(),
        cx,
    );
    stage.keeper.tell(turn_ended(&session_s(), &s2(), &s1()));
    listed_for(
        &mut stage,
        &review_s(),
        &s1(),
        &s2(),
        the_list_of_s(),
        &mut cx,
    );
    read_files(
        &mut stage,
        Some(&s1()),
        Some(&s2()),
        &[
            ("b.txt", "b\n", "b2\n"),
            ("h.txt", "h\n", "h2\n"),
            ("s.txt", "s\n", "s2\n"),
        ],
        &mut cx,
    );
    assert!(marked(&stage, "s.txt", &mut cx));

    // The turn of `t`, a session the pane holds no review of, ends: its change is listed by
    // `CompareVersions` over the review that stays shown, so it carries no credits.
    stage.keeper.tell(turn_ended(&session_t(), &s3(), &s2()));
    let asked = stage.keeper.expect(
        Request::CompareVersions {
            workspace: id('1'),
            from: s2(),
            to: Some(s3()),
        },
        &mut cx,
    );
    stage.keeper.reply(
        asked,
        Ok(Reply::Compared(vec![file("s.txt", FileStatus::Modified)])),
    );
    read_files(
        &mut stage,
        Some(&s2()),
        Some(&s3()),
        &[("s.txt", "s2\n", "s3\n")],
        &mut cx,
    );
    assert_eq!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.review_session().cloned()),
        Some(session_s())
    );
    assert!(!marked(&stage, "s.txt", &mut cx));
}

#[gpui_kit::test]
fn the_same_change_listed_for_another_session_s_review_is_asked_again_with_its_own_credits(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with(
        vec![
            (review_s(), Some(session_s())),
            (review_t(), Some(session_t())),
        ],
        &review_s(),
        Vec::new(),
        cx,
    );
    stage.keeper.tell(turn_ended(&session_s(), &s2(), &s1()));
    listed_for(
        &mut stage,
        &review_s(),
        &s1(),
        &s2(),
        the_list_of_s(),
        &mut cx,
    );
    read_files(
        &mut stage,
        Some(&s1()),
        Some(&s2()),
        &[
            ("b.txt", "b\n", "b2\n"),
            ("h.txt", "h\n", "h2\n"),
            ("s.txt", "s\n", "s2\n"),
        ],
        &mut cx,
    );

    stage.keeper.tell(turn_ended(&session_t(), &s2(), &s1()));
    listed_for(
        &mut stage,
        &review_t(),
        &s1(),
        &s2(),
        vec![
            credited("a.txt", vec![session_t()]),
            credited("s.txt", vec![session_t(), session_s()]),
        ],
        &mut cx,
    );
    read_files(
        &mut stage,
        Some(&s1()),
        Some(&s2()),
        &[("a.txt", "a\n", "a2\n"), ("s.txt", "s\n", "s2\n")],
        &mut cx,
    );
    assert_eq!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.review_session().cloned()),
        Some(session_t())
    );
    assert!(marked(&stage, "s.txt", &mut cx));
    assert!(!marked(&stage, "a.txt", &mut cx));
}

#[gpui_kit::test]
fn moving_to_another_review_takes_the_marks_of_a_list_made_for_the_review_before_away(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with(
        vec![
            (review_s(), Some(session_s())),
            (review_t(), Some(session_t())),
        ],
        &review_s(),
        Vec::new(),
        cx,
    );
    let sessions = vec![
        session('5', "Claude Code 1", SessionState::Works, Some(review_s())),
        session('6', "Claude Code 2", SessionState::Works, Some(review_t())),
    ];
    stage.keeper.tell_agents(agents_of(sessions.clone()));
    wait_until(&mut cx, "the sessions in the pane", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.sessions() == sessions)
    });
    stage.keeper.tell(turn_ended(&session_s(), &s2(), &s1()));
    listed_for(
        &mut stage,
        &review_s(),
        &s1(),
        &s2(),
        the_list_of_s(),
        &mut cx,
    );
    read_files(
        &mut stage,
        Some(&s1()),
        Some(&s2()),
        &[
            ("b.txt", "b\n", "b2\n"),
            ("h.txt", "h\n", "h2\n"),
            ("s.txt", "s\n", "s2\n"),
        ],
        &mut cx,
    );
    assert!(marked(&stage, "s.txt", &mut cx));

    // The reviewer moves to `t`'s review. The list stays, but its credits were made for `s`'s
    // review: read as `t`'s, they would mark a file `s` also changed, which no list for `t` says.
    click("review-session-picker", &mut cx);
    click("review-session-1", &mut cx);
    assert_eq!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.review_session().cloned()),
        Some(session_t())
    );
    assert!(!marked(&stage, "s.txt", &mut cx));
}

#[gpui_kit::test]
fn the_uncommitted_changes_of_a_review_mark_the_file_another_session_changed_too(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with(
        vec![(review_s(), Some(session_s()))],
        &review_s(),
        vec![
            credited("b.txt", vec![session_s()]),
            credited("h.txt", Vec::new()),
            credited("s.txt", vec![session_t(), session_s()]),
            credited("t.txt", vec![session_t()]),
        ],
        cx,
    );
    click("review-history-uncommitted", &mut cx);
    read_files(
        &mut stage,
        Some(&c2()),
        None,
        &[
            ("b.txt", "b\n", "b2\n"),
            ("h.txt", "h\n", "h2\n"),
            ("s.txt", "s\n", "s2\n"),
            ("t.txt", "t\n", "t2\n"),
        ],
        &mut cx,
    );
    assert_eq!(shown(&stage, &mut cx), ChangeShown::Uncommitted);
    assert!(marked(&stage, "s.txt", &mut cx));
    assert!(!marked(&stage, "b.txt", &mut cx));
    assert!(!marked(&stage, "t.txt", &mut cx));
    assert!(!marked(&stage, "h.txt", &mut cx));
    let h = row_of(&stage, "h.txt", &mut cx);
    assert!(drawn(&format!("review-no-agent-{h}"), &mut cx).is_some());
    let s = row_of(&stage, "s.txt", &mut cx);
    assert!(drawn(&format!("review-no-agent-{s}"), &mut cx).is_none());
}

#[gpui_kit::test]
fn the_review_of_no_agent_marks_no_file_whoever_changed_it(cx: &mut TestAppContext) {
    let (mut stage, mut cx) = open_with(
        vec![(review_of_no_agent(), None)],
        &review_of_no_agent(),
        vec![credited("s.txt", vec![session_s(), session_t()])],
        cx,
    );
    click("review-history-uncommitted", &mut cx);
    read_files(
        &mut stage,
        Some(&c2()),
        None,
        &[("s.txt", "s\n", "s2\n")],
        &mut cx,
    );
    assert!(!marked(&stage, "s.txt", &mut cx));
}

#[gpui_kit::test]
fn a_comparison_the_keeper_refuses_for_a_review_shows_no_file_and_no_mark_and_says_why(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with(
        vec![(review_s(), Some(session_s()))],
        &review_s(),
        Vec::new(),
        cx,
    );
    stage.keeper.tell(turn_ended(&session_s(), &s2(), &s1()));
    let asked = stage
        .keeper
        .expect(compare_for(&review_s(), s1(), s2()), &mut cx);
    stage
        .keeper
        .reply(asked, Err(Refusal::UnknownRevision { revision: s2() }));
    wait_until(&mut cx, "the notice", |cx| {
        stage.pane.read_with(cx, |pane, _| pane.notice().is_some())
    });
    assert_eq!(paths(&stage.pane, &mut cx), Vec::<String>::new());
    assert!(!stage.pane.read_with(&cx, |pane, _| {
        pane.changed_by_another_too(&PathBuf::from("s.txt"))
    }));
    stage.keeper.tell(turn_ended(&session_s(), &s2(), &s1()));
    listed_for(
        &mut stage,
        &review_s(),
        &s1(),
        &s2(),
        the_list_of_s(),
        &mut cx,
    );
    read_files(
        &mut stage,
        Some(&s1()),
        Some(&s2()),
        &[
            ("b.txt", "b\n", "b2\n"),
            ("h.txt", "h\n", "h2\n"),
            ("s.txt", "s\n", "s2\n"),
        ],
        &mut cx,
    );
    assert!(marked(&stage, "s.txt", &mut cx));
}

fn the_list_of_s_is_shown(cx: &mut TestAppContext) -> (Stage, VisualTestContext) {
    the_list_of_s_is_shown_among(
        vec![
            (review_s(), Some(session_s())),
            (review_t(), Some(session_t())),
        ],
        &review_s(),
        cx,
    )
}

fn the_list_of_s_is_shown_among(
    reviews: Vec<(ReviewId, Option<SessionId>)>,
    first_shown: &ReviewId,
    cx: &mut TestAppContext,
) -> (Stage, VisualTestContext) {
    let (mut stage, mut cx) = open_with(reviews, first_shown, Vec::new(), cx);
    let sessions = vec![
        session('5', "Claude Code 1", SessionState::Works, Some(review_s())),
        session('6', "Claude Code 2", SessionState::Works, Some(review_t())),
    ];
    stage.keeper.tell_agents(agents_of(sessions.clone()));
    wait_until(&mut cx, "the sessions in the pane", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.sessions() == sessions)
    });
    stage.keeper.tell(turn_ended(&session_s(), &s2(), &s1()));
    listed_for(
        &mut stage,
        &review_s(),
        &s1(),
        &s2(),
        the_list_of_s(),
        &mut cx,
    );
    read_files(
        &mut stage,
        Some(&s1()),
        Some(&s2()),
        &[
            ("b.txt", "b\n", "b2\n"),
            ("h.txt", "h\n", "h2\n"),
            ("s.txt", "s\n", "s2\n"),
        ],
        &mut cx,
    );
    assert!(marked(&stage, "s.txt", &mut cx));
    (stage, cx)
}

#[gpui_kit::test]
fn moving_to_another_review_lists_the_change_shown_again_for_that_review(cx: &mut TestAppContext) {
    let (mut stage, mut cx) = the_list_of_s_is_shown(cx);

    click("review-session-picker", &mut cx);
    click("review-session-1", &mut cx);
    listed_for(
        &mut stage,
        &review_t(),
        &s1(),
        &s2(),
        vec![
            credited("a.txt", vec![session_t()]),
            credited("s.txt", vec![session_t(), session_s()]),
        ],
        &mut cx,
    );
    read_files(
        &mut stage,
        Some(&s1()),
        Some(&s2()),
        &[("a.txt", "a\n", "a2\n"), ("s.txt", "s\n", "s2\n")],
        &mut cx,
    );
    assert_eq!(
        shown(&stage, &mut cx),
        ChangeShown::Since {
            from: Some(s1()),
            to: s2()
        }
    );
    assert!(marked(&stage, "s.txt", &mut cx));
    assert!(!marked(&stage, "a.txt", &mut cx));
}

#[gpui_kit::test]
fn moving_to_the_review_of_no_agent_lists_the_change_shown_again_with_no_credits(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with(
        vec![
            (review_s(), Some(session_s())),
            (review_of_no_agent(), None),
        ],
        &review_of_no_agent(),
        Vec::new(),
        cx,
    );
    let sessions = vec![session(
        '5',
        "Claude Code 1",
        SessionState::Works,
        Some(review_s()),
    )];
    stage.keeper.tell_agents(agents_of(sessions.clone()));
    wait_until(&mut cx, "the sessions in the pane", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.sessions() == sessions)
    });
    stage.keeper.tell(turn_ended(&session_s(), &s2(), &s1()));
    listed_for(
        &mut stage,
        &review_s(),
        &s1(),
        &s2(),
        the_list_of_s(),
        &mut cx,
    );
    read_files(
        &mut stage,
        Some(&s1()),
        Some(&s2()),
        &[
            ("b.txt", "b\n", "b2\n"),
            ("h.txt", "h\n", "h2\n"),
            ("s.txt", "s\n", "s2\n"),
        ],
        &mut cx,
    );

    click("review-session-picker", &mut cx);
    click("review-session-1", &mut cx);
    let asked = stage.keeper.expect(
        Request::CompareVersions {
            workspace: id('1'),
            from: s1(),
            to: Some(s2()),
        },
        &mut cx,
    );
    stage.keeper.reply(
        asked,
        Ok(Reply::Compared(vec![
            file("b.txt", FileStatus::Modified),
            file("t.txt", FileStatus::Modified),
        ])),
    );
    read_files(
        &mut stage,
        Some(&s1()),
        Some(&s2()),
        &[("b.txt", "b\n", "b2\n"), ("t.txt", "t\n", "t2\n")],
        &mut cx,
    );
    assert_eq!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.review_session().cloned()),
        None
    );
    assert!(!marked(&stage, "b.txt", &mut cx));
}

#[gpui_kit::test]
fn a_turn_end_back_to_the_review_asked_for_before_is_asked_again_while_another_review_s_list_is_on_its_way(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = the_list_of_s_is_shown(cx);

    // `t`'s list is still in flight when `s`'s turn ends: `s`'s list is shown, `t`'s late answer is
    // not.
    stage.keeper.tell(turn_ended(&session_t(), &s2(), &s1()));
    let for_t = stage
        .keeper
        .expect(compare_for(&review_t(), s1(), s2()), &mut cx);
    stage.keeper.tell(turn_ended(&session_s(), &s2(), &s1()));
    let for_s = stage
        .keeper
        .expect(compare_for(&review_s(), s1(), s2()), &mut cx);
    stage.keeper.reply(
        for_t,
        Ok(Reply::ComparedForReview(vec![credited(
            "a.txt",
            vec![session_t()],
        )])),
    );
    stage
        .keeper
        .reply(for_s, Ok(Reply::ComparedForReview(the_list_of_s())));
    read_files(
        &mut stage,
        Some(&s1()),
        Some(&s2()),
        &[
            ("b.txt", "b\n", "b2\n"),
            ("h.txt", "h\n", "h2\n"),
            ("s.txt", "s\n", "s2\n"),
        ],
        &mut cx,
    );
    assert_eq!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.review_session().cloned()),
        Some(session_s())
    );
    assert!(marked(&stage, "s.txt", &mut cx));
}

fn the_files_of_the_change() -> Vec<yaar_link::ChangedFile> {
    vec![
        file("b.txt", FileStatus::Modified),
        file("h.txt", FileStatus::Modified),
        file("s.txt", FileStatus::Modified),
        file("t.txt", FileStatus::Modified),
    ]
}

fn the_list_of_s_and_then_the_review_of_no_agent(
    cx: &mut TestAppContext,
) -> (Stage, VisualTestContext) {
    let (mut stage, mut cx) = the_list_of_s_is_shown_among(
        vec![
            (review_s(), Some(session_s())),
            (review_t(), Some(session_t())),
            (review_of_no_agent(), None),
        ],
        &review_of_no_agent(),
        cx,
    );
    click("review-session-picker", &mut cx);
    click("review-session-2", &mut cx);
    let asked = stage.keeper.expect(
        Request::CompareVersions {
            workspace: id('1'),
            from: s1(),
            to: Some(s2()),
        },
        &mut cx,
    );
    stage
        .keeper
        .reply(asked, Ok(Reply::Compared(the_files_of_the_change())));
    read_files(
        &mut stage,
        Some(&s1()),
        Some(&s2()),
        &[
            ("b.txt", "b\n", "b2\n"),
            ("h.txt", "h\n", "h2\n"),
            ("s.txt", "s\n", "s2\n"),
            ("t.txt", "t\n", "t2\n"),
        ],
        &mut cx,
    );
    assert!(!marked(&stage, "s.txt", &mut cx));
    (stage, cx)
}

fn back_to_the_list_of_s(stage: &mut Stage, cx: &mut VisualTestContext) {
    listed_for(stage, &review_s(), &s1(), &s2(), the_list_of_s(), cx);
    read_files(
        stage,
        Some(&s1()),
        Some(&s2()),
        &[
            ("b.txt", "b\n", "b2\n"),
            ("h.txt", "h\n", "h2\n"),
            ("s.txt", "s\n", "s2\n"),
        ],
        cx,
    );
    assert_eq!(
        stage
            .pane
            .read_with(cx, |pane, _| pane.review_session().cloned()),
        Some(session_s())
    );
    assert!(marked(stage, "s.txt", cx));
    assert!(!marked(stage, "b.txt", cx));
}

#[gpui_kit::test]
fn a_click_back_from_the_review_of_no_agent_lists_the_change_again_for_the_session_s_review(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = the_list_of_s_and_then_the_review_of_no_agent(cx);

    click("review-session-picker", &mut cx);
    click("review-session-0", &mut cx);
    back_to_the_list_of_s(&mut stage, &mut cx);
}

#[gpui_kit::test]
fn shift_a_back_from_the_review_of_no_agent_lists_the_change_again_for_the_session_s_review(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = the_list_of_s_and_then_the_review_of_no_agent(cx);

    cx.simulate_keystrokes("shift-a");
    back_to_the_list_of_s(&mut stage, &mut cx);
}

#[gpui_kit::test]
fn the_cycle_of_shift_a_through_every_review_ends_on_the_list_of_the_review_it_started_at(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = the_list_of_s_is_shown_among(
        vec![
            (review_s(), Some(session_s())),
            (review_t(), Some(session_t())),
            (review_of_no_agent(), None),
        ],
        &review_of_no_agent(),
        cx,
    );
    click("review-session-picker", &mut cx);
    click("review-session-0", &mut cx);
    cx.simulate_keystrokes("shift-a");
    listed_for(
        &mut stage,
        &review_t(),
        &s1(),
        &s2(),
        vec![credited("a.txt", vec![session_t()])],
        &mut cx,
    );
    read_files(
        &mut stage,
        Some(&s1()),
        Some(&s2()),
        &[("a.txt", "a\n", "a2\n")],
        &mut cx,
    );
    cx.simulate_keystrokes("shift-a");
    let asked = stage.keeper.expect(
        Request::CompareVersions {
            workspace: id('1'),
            from: s1(),
            to: Some(s2()),
        },
        &mut cx,
    );
    stage
        .keeper
        .reply(asked, Ok(Reply::Compared(the_files_of_the_change())));
    read_files(
        &mut stage,
        Some(&s1()),
        Some(&s2()),
        &[
            ("b.txt", "b\n", "b2\n"),
            ("h.txt", "h\n", "h2\n"),
            ("s.txt", "s\n", "s2\n"),
            ("t.txt", "t\n", "t2\n"),
        ],
        &mut cx,
    );
    cx.simulate_keystrokes("shift-a");
    back_to_the_list_of_s(&mut stage, &mut cx);
}

#[gpui_kit::test]
fn reviews_that_arrive_after_a_turn_end_chose_a_session_s_review_list_the_change_for_it(
    cx: &mut TestAppContext,
) {
    // When `s`'s turn ends the pane holds only the review of no agent: the agents model already
    // names `s`'s review, the Keeper's list of reviews does not yet.
    let (mut stage, mut cx) = open_with(
        vec![(review_of_no_agent(), None)],
        &review_of_no_agent(),
        Vec::new(),
        cx,
    );
    let sessions = vec![session(
        '5',
        "Claude Code 1",
        SessionState::Works,
        Some(review_s()),
    )];
    stage.keeper.tell_agents(agents_of(sessions.clone()));
    wait_until(&mut cx, "the sessions in the pane", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.sessions() == sessions)
    });
    stage.keeper.tell(turn_ended(&session_s(), &s2(), &s1()));
    listed_for(
        &mut stage,
        &review_s(),
        &s1(),
        &s2(),
        the_list_of_s(),
        &mut cx,
    );
    read_files(
        &mut stage,
        Some(&s1()),
        Some(&s2()),
        &[
            ("b.txt", "b\n", "b2\n"),
            ("h.txt", "h\n", "h2\n"),
            ("s.txt", "s\n", "s2\n"),
        ],
        &mut cx,
    );

    stage
        .keeper
        .tell(Fact::ReviewsChanged { workspace: id('1') });
    let asked = stage.keeper.expect(
        Request::CompareVersions {
            workspace: id('1'),
            from: s1(),
            to: Some(s2()),
        },
        &mut cx,
    );
    stage
        .keeper
        .reply(asked, Ok(Reply::Compared(the_files_of_the_change())));
    read_files(
        &mut stage,
        Some(&s1()),
        Some(&s2()),
        &[
            ("b.txt", "b\n", "b2\n"),
            ("h.txt", "h\n", "h2\n"),
            ("s.txt", "s\n", "s2\n"),
            ("t.txt", "t\n", "t2\n"),
        ],
        &mut cx,
    );

    stage.keeper.set_reviews(reviews_of(vec![
        common::review_with(review_of_no_agent(), None, Some(s1())),
        common::review_with(review_s(), Some(session_s()), Some(s1())),
    ]));
    stage
        .keeper
        .tell(Fact::ReviewsChanged { workspace: id('1') });
    back_to_the_list_of_s(&mut stage, &mut cx);
}

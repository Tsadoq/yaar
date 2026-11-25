//! Component test: the rounds of a review and the threads that moved with the code.
//! A negative clause is read after a barrier: a later request whose arrival shows the earlier ones
//! arrived.

mod common;

use std::path::PathBuf;

use gpui_kit::{Modifiers, TestAppContext, VisualTestContext};
use yaar_diff::Side;
use yaar_link::{
    Anchor, Author, ChangedFile, Comment, CommentId, Fact, FileStatus, History, HistoryEntry,
    Lines, Notice, Placed, Reply, Request, ReviewId, Round, SentBy, SessionState, Side as LinkSide,
    Thread, ThreadId, ThreadState, TurnId, WorkspaceReviews,
};
use yaar_review_pane::{ChangeShown, PaneRow, Position, RoundShown};

use common::{
    COMMIT, Stage, agents_of, bounds_of_row, drawn, file, id, open_staged, repository, review_with,
    reviews_of, session, session_id, since_main, texts, wait_for_files, wait_until, workspace,
};

fn version(digit: char) -> String {
    digit.to_string().repeat(40)
}

fn review_id() -> ReviewId {
    ReviewId::new("1".repeat(32))
}

fn story() -> PathBuf {
    PathBuf::from("story.txt")
}

fn turn(digit: char) -> HistoryEntry {
    HistoryEntry::Turn {
        turn: TurnId::new(digit.to_string().repeat(32)),
        session: session_id('a'),
        state: Some(version(digit)),
    }
}

fn history_of(states: &str) -> History {
    History {
        workspace: id('1'),
        start: Some(COMMIT.to_owned()),
        entries: states.chars().map(turn).collect(),
    }
}

fn round(number: u32, state: Option<char>, edited: bool) -> Round {
    Round {
        number,
        state: state.map(version),
        sent_by: SentBy::Delivered,
        edited,
        turn: None,
    }
}

fn round_in_turn(number: u32, state: char, turn: char) -> Round {
    Round {
        turn: Some(TurnId::new(turn.to_string().repeat(32))),
        ..round(number, Some(state), false)
    }
}

fn review_of_a(rounds: Vec<Round>, threads: Vec<Thread>) -> WorkspaceReviews {
    let mut review = review_with(review_id(), Some(session_id('a')), None);
    review.rounds = rounds;
    review.threads = threads;
    reviews_of(vec![review])
}

fn comment(digit: char) -> Comment {
    Comment {
        id: CommentId::new(digit.to_string().repeat(32)),
        author: Author::Reviewer,
        text: format!("comment {digit}"),
        round: Some(1),
    }
}

fn thread_id(digit: char) -> ThreadId {
    ThreadId::new(digit.to_string().repeat(32))
}

fn thread_at(digit: char, tree: Option<String>, line: u32, text: &str) -> Thread {
    Thread {
        id: thread_id(digit),
        anchor: Anchor {
            tree,
            path: story(),
            side: LinkSide::New,
            lines: Lines {
                first: line,
                last: line,
            },
            text: text.to_owned(),
        },
        resolved: false,
        addressed: false,
        comments: vec![comment(digit)],
        state: ThreadState::Sent,
        now: None,
    }
}

fn carried(thread: Thread, tree: String, first: u32, last: u32, notice: Option<Notice>) -> Thread {
    Thread {
        now: Some(Placed {
            tree,
            path: story(),
            side: LinkSide::New,
            lines: Lines { first, last },
            notice,
        }),
        ..thread
    }
}

/// The history is set after the changes are answered: a pane without its changes shows the last
/// turn.
fn open_on_story(
    reviews: WorkspaceReviews,
    history: History,
    old: &str,
    new: &str,
    cx: &mut TestAppContext,
) -> (Stage, VisualTestContext) {
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature")],
        |keeper| {
            keeper.set_agents(vec![agents_of(vec![session(
                'a',
                "Claude Code 1",
                SessionState::Idle,
                Some(review_id()),
            )])]);
            keeper.set_reviews(reviews);
        },
        cx,
    );
    let changes = since_main(Vec::new(), vec![file("story.txt", FileStatus::Modified)]);
    stage.keeper.show(&changes, |_| texts(old, new), &mut cx);
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
    (stage, cx)
}

fn is_drawn(selector: &str, cx: &mut VisualTestContext) -> bool {
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    drawn(selector, cx).is_some()
}

fn click(selector: &str, cx: &mut VisualTestContext) {
    assert!(is_drawn(selector, cx), "{selector} is not drawn");
    let at = drawn(selector, cx).expect("it was drawn").center();
    cx.simulate_click(at, Modifiers::none());
}

fn tell_reviews(stage: &mut Stage, reviews: WorkspaceReviews, cx: &mut VisualTestContext) {
    let held = (
        reviews.reviews[0].rounds.clone(),
        reviews.reviews[0].threads.clone(),
    );
    stage.keeper.set_reviews(reviews);
    stage
        .keeper
        .tell(Fact::ReviewsChanged { workspace: id('1') });
    wait_until(cx, "the reviews as told", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.review()
                .is_some_and(|review| (&review.rounds, &review.threads) == (&held.0, &held.1))
        })
    });
}

fn history_grows(stage: &mut Stage, history: History, cx: &mut VisualTestContext) {
    let asked = stage.keeper.history_asked();
    stage.keeper.set_history(history.clone());
    stage
        .keeper
        .tell(Fact::Workspaces(vec![repository(vec![workspace(
            '1', "feature",
        )])]));
    wait_until(cx, "the longer history in the pane", |cx| {
        stage.keeper.history_asked() > asked
            && stage
                .pane
                .read_with(cx, |pane, _| pane.history() == Some(&history))
    });
}

fn notice(stage: &Stage, cx: &mut VisualTestContext) -> Option<String> {
    stage.pane.read_with(cx, |pane, _| pane.notice())
}

fn rounds(stage: &Stage, cx: &mut VisualTestContext) -> Vec<RoundShown> {
    stage.pane.read_with(cx, |pane, _| pane.rounds_shown())
}

fn shown(stage: &Stage, cx: &mut VisualTestContext) -> ChangeShown {
    stage.pane.read_with(cx, |pane, _| pane.change_shown())
}

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

fn read_story(
    stage: &mut Stage,
    old: Option<String>,
    new: Option<String>,
    texts_of: (&str, &str),
    cx: &mut VisualTestContext,
) {
    let asked = stage.keeper.expect(
        Request::ReadVersions {
            workspace: id('1'),
            old,
            old_path: story(),
            new,
            path: story(),
        },
        cx,
    );
    stage
        .keeper
        .reply(asked, texts(texts_of.0, texts_of.1).map(Reply::File));
}

fn focus_the_rows(stage: &Stage, cx: &mut VisualTestContext) {
    cx.update(|window, cx| stage.focus.focus(window, cx));
    cx.update(|window, _| assert!(stage.focus.is_focused(window)));
}

#[gpui_kit::test]
fn a_round_shows_as_sent_until_the_agents_first_edit_and_each_round_shows_what_changed_since_its_comments_were_sent(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_on_story(
        review_of_a(vec![round(1, Some('5'), false)], Vec::new()),
        history_of("567"),
        "a\nb\n",
        "a\nc\n",
        cx,
    );

    assert_eq!(
        rounds(&stage, &mut cx),
        vec![RoundShown {
            number: 1,
            sent: true,
            state: Some(version('5')),
        }]
    );
    assert!(is_drawn("review-round-1:sent", &mut cx));
    assert!(!is_drawn("review-round-1:answered", &mut cx));
    assert!(is_drawn("review-round-1-since", &mut cx));

    tell_reviews(
        &mut stage,
        review_of_a(vec![round(1, Some('5'), true)], Vec::new()),
        &mut cx,
    );
    assert_eq!(
        rounds(&stage, &mut cx),
        vec![RoundShown {
            number: 1,
            sent: false,
            state: Some(version('5')),
        }]
    );
    assert!(is_drawn("review-round-1:answered", &mut cx));
    assert!(!is_drawn("review-round-1:sent", &mut cx));

    click("review-round-1-since", &mut cx);
    assert_eq!(
        shown(&stage, &mut cx),
        ChangeShown::Since {
            from: Some(version('5')),
            to: version('7'),
        }
    );
    compared(
        &mut stage,
        version('5'),
        Some(version('7')),
        Vec::new(),
        &mut cx,
    );

    click("review-history-0", &mut cx);
    compared(
        &mut stage,
        COMMIT.to_owned(),
        Some(version('5')),
        Vec::new(),
        &mut cx,
    );
    assert_eq!(
        shown(&stage, &mut cx),
        ChangeShown::Entries { first: 0, last: 0 }
    );
    focus_the_rows(&stage, &mut cx);
    cx.simulate_keystrokes("shift-s");
    assert_eq!(
        shown(&stage, &mut cx),
        ChangeShown::Since {
            from: Some(version('5')),
            to: version('7'),
        }
    );
    compared(
        &mut stage,
        version('5'),
        Some(version('7')),
        Vec::new(),
        &mut cx,
    );

    tell_reviews(
        &mut stage,
        review_of_a(
            vec![round(1, Some('5'), false), round(2, None, false)],
            Vec::new(),
        ),
        &mut cx,
    );
    assert_eq!(
        rounds(&stage, &mut cx)
            .iter()
            .map(|round| (round.number, round.sent))
            .collect::<Vec<_>>(),
        [(1, true), (2, true)]
    );
    assert!(is_drawn("review-round-2:sent", &mut cx));
    assert!(drawn("review-round-1:sent", &mut cx).is_some());
    assert!(drawn("review-round-1:answered", &mut cx).is_none());
    assert!(drawn("review-round-1-since", &mut cx).is_some());
    assert!(
        drawn("review-round-2-since", &mut cx).is_none(),
        "a round with no state has no control"
    );

    tell_reviews(
        &mut stage,
        review_of_a(
            vec![round(1, Some('5'), false), round(2, Some('6'), false)],
            Vec::new(),
        ),
        &mut cx,
    );
    assert!(is_drawn("review-round-2-since", &mut cx));

    tell_reviews(
        &mut stage,
        review_of_a(
            vec![round(1, Some('5'), false), round(2, Some('6'), true)],
            Vec::new(),
        ),
        &mut cx,
    );
    assert_eq!(
        rounds(&stage, &mut cx)
            .iter()
            .map(|round| (round.number, round.sent))
            .collect::<Vec<_>>(),
        [(1, false), (2, false)]
    );
    assert!(is_drawn("review-round-1:answered", &mut cx));
    assert!(drawn("review-round-2:answered", &mut cx).is_some());

    focus_the_rows(&stage, &mut cx);
    let round_two = ChangeShown::Since {
        from: Some(version('6')),
        to: version('7'),
    };
    let round_one = ChangeShown::Since {
        from: Some(version('5')),
        to: version('7'),
    };
    for (wanted, from) in [(&round_two, '6'), (&round_one, '5'), (&round_two, '6')] {
        cx.simulate_keystrokes("shift-s");
        assert_eq!(&shown(&stage, &mut cx), wanted);
        compared(
            &mut stage,
            version(from),
            Some(version('7')),
            Vec::new(),
            &mut cx,
        );
    }
}

#[gpui_kit::test]
fn a_thread_that_moved_to_rewritten_code_says_that_the_code_changed_and_shows_on_request_the_line_the_comment_was_written_on(
    cx: &mut TestAppContext,
) {
    let moved = carried(
        thread_at('7', Some(version('4')), 3, "old line"),
        version('5'),
        3,
        4,
        Some(Notice::Changed),
    );
    let mut written = thread_at('8', Some(version('4')), 5, "gone line\nsecond line");
    written.anchor.lines = Lines { first: 5, last: 6 };
    let removed = carried(written, version('5'), 5, 5, Some(Notice::Deleted));
    let (stage, mut cx) = open_on_story(
        review_of_a(vec![round(1, Some('5'), true)], vec![moved, removed]),
        history_of("5"),
        "a\nb\nold line\nd\ne\n",
        "a\nb\nnew one\nnew two\ne\n",
        cx,
    );
    cx.simulate_resize(gpui_kit::size(common::WINDOW.width, gpui_kit::px(760.)));
    let (the, other) = (thread_id('7'), thread_id('8'));
    let all_rows = |stage: &Stage, cx: &mut VisualTestContext| -> Vec<PaneRow> {
        stage.pane.read_with(cx, |pane, _| pane.rows().to_vec())
    };
    let index_of_new_line = |line: u32, stage: &Stage, cx: &mut VisualTestContext| {
        all_rows(stage, cx)
            .iter()
            .position(
                |row| matches!(row, PaneRow::Diff { row, .. } if row.line(Side::New) == Some(line)),
            )
            .unwrap_or_else(|| panic!("no row shows new line {}", line + 1))
    };

    let line_four = index_of_new_line(3, &stage, &mut cx);
    assert_eq!(
        all_rows(&stage, &mut cx)[line_four + 1],
        PaneRow::Thread {
            thread: the.clone()
        }
    );

    assert!(is_drawn(
        &format!("review-thread-notice-{the}:changed"),
        &mut cx
    ));
    assert!(drawn(&format!("review-thread-notice-{other}:deleted"), &mut cx).is_some());
    assert!(drawn(&format!("review-thread-notice-{the}:deleted"), &mut cx).is_none());

    let on_line = bounds_of_row(line_four, &mut cx).center();
    cx.simulate_click(on_line, Modifiers::none());
    cx.update(|window, _| assert!(stage.focus.is_focused(window)));
    assert_eq!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.position().cloned()),
        Some(Position {
            path: story(),
            side: Side::New,
            line: 3,
        })
    );
    let original = format!("review-thread-original-text-{the}");
    assert!(
        !stage
            .pane
            .read_with(&cx, |pane, _| pane.shows_original(&the))
    );
    assert!(!is_drawn(&original, &mut cx));
    cx.simulate_keystrokes("w");
    assert!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.shows_original(&the))
    );
    assert!(is_drawn(&original, &mut cx));
    assert_eq!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.original_text(&the)),
        Some("old line".to_owned())
    );
    assert!(
        !stage
            .pane
            .read_with(&cx, |pane, _| pane.shows_original(&other)),
        "the other thread is not on that line"
    );
    cx.simulate_keystrokes("w");
    assert!(
        !stage
            .pane
            .read_with(&cx, |pane, _| pane.shows_original(&the))
    );
    assert!(!is_drawn(&original, &mut cx));

    click(&format!("review-thread-original-{the}"), &mut cx);
    assert!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.shows_original(&the))
    );
    assert!(is_drawn(&original, &mut cx));
    click(&format!("review-thread-original-{the}"), &mut cx);
    assert!(
        !stage
            .pane
            .read_with(&cx, |pane, _| pane.shows_original(&the))
    );
    assert!(!is_drawn(&original, &mut cx));

    // The test platform draws no readable glyph but lays text out monospace: drawn width counts
    // characters.
    click(&format!("review-thread-original-{the}"), &mut cx);
    click(&format!("review-thread-original-{other}"), &mut cx);
    let text_of = |thread: &ThreadId, cx: &mut VisualTestContext| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.original_text(thread))
            .expect("the line is shown")
    };
    assert_eq!(text_of(&other, &mut cx), "gone line | second line");
    let advance = |thread: &ThreadId, cx: &mut VisualTestContext| {
        assert!(is_drawn(
            &format!("review-thread-original-text-{thread}"),
            cx
        ));
        let width = drawn(&format!("review-thread-original-text-{thread}"), cx)
            .expect("it was drawn")
            .size
            .width;
        f32::from(width) / text_of(thread, cx).chars().count() as f32
    };
    let (one, two) = (advance(&the, &mut cx), advance(&other, &mut cx));
    assert!(
        (one - two).abs() < 0.2,
        "a character is {one} wide in one text and {two} in the other"
    );
}

#[gpui_kit::test]
fn a_thread_carried_to_a_version_the_tree_does_not_show_is_under_threads_elsewhere_and_on_the_lines_it_was_carried_to_once_that_version_is_shown(
    cx: &mut TestAppContext,
) {
    let thread = carried(
        thread_at('7', Some(version('1')), 2, "b"),
        version('3'),
        2,
        2,
        Some(Notice::Changed),
    );
    let (mut stage, mut cx) = open_on_story(
        review_of_a(vec![round(1, Some('1'), true)], vec![thread]),
        history_of("123"),
        "a\nb\nc\n",
        "a\nB\nc\n",
        cx,
    );
    let the = thread_id('7');
    let notice = format!("review-thread-notice-{the}:changed");
    let rows_of = |stage: &Stage, cx: &mut VisualTestContext| -> Vec<PaneRow> {
        stage.pane.read_with(cx, |pane, _| pane.rows().to_vec())
    };
    let elsewhere = |stage: &Stage, cx: &mut VisualTestContext| {
        let rows = rows_of(stage, cx);
        let at = rows
            .iter()
            .position(|row| *row == PaneRow::ThreadsElsewhere);
        at.is_some_and(|at| {
            rows[at + 1..].contains(&PaneRow::Thread {
                thread: the.clone(),
            })
        })
    };
    assert!(!elsewhere(&stage, &mut cx));
    assert!(is_drawn(&notice, &mut cx));

    let select =
        |stage: &mut Stage, entry: usize, from: String, to: String, cx: &mut VisualTestContext| {
            click(&format!("review-history-{entry}"), cx);
            compared(
                stage,
                from.clone(),
                Some(to.clone()),
                vec![file("story.txt", FileStatus::Modified)],
                cx,
            );
            read_story(stage, Some(from), Some(to), ("a\nb\nc\n", "a\nB\nc\n"), cx);
            wait_for_files(&stage.pane, cx);
        };
    select(&mut stage, 1, version('1'), version('2'), &mut cx);
    assert!(elsewhere(&stage, &mut cx));
    assert!(!is_drawn(&notice, &mut cx));

    select(&mut stage, 2, version('2'), version('3'), &mut cx);
    assert!(!elsewhere(&stage, &mut cx));
    assert!(is_drawn(&notice, &mut cx));
    let line_two = rows_of(&stage, &mut cx)
        .iter()
        .position(|row| matches!(row, PaneRow::Diff { row, .. } if row.line(Side::New) == Some(1)))
        .expect("a row shows new line 2");
    assert_eq!(
        rows_of(&stage, &mut cx)[line_two + 1],
        PaneRow::Thread {
            thread: the.clone()
        }
    );

    let original = format!("review-thread-original-text-{the}");
    click(&format!("review-thread-original-{the}"), &mut cx);
    assert!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.shows_original(&the))
    );
    assert!(is_drawn(&original, &mut cx));

    select(&mut stage, 0, COMMIT.to_owned(), version('1'), &mut cx);
    assert!(!elsewhere(&stage, &mut cx));
    assert!(!is_drawn(&notice, &mut cx));
    assert!(!is_drawn(&original, &mut cx));
    assert!(
        !stage
            .pane
            .read_with(&cx, |pane, _| pane.shows_original(&the))
    );
    assert!(rows_of(&stage, &mut cx).contains(&PaneRow::Thread {
        thread: the.clone()
    }));
}

#[gpui_kit::test]
fn w_where_no_moved_thread_is_and_shift_s_with_no_round_that_has_a_state_say_so_in_the_notice_and_send_nothing(
    cx: &mut TestAppContext,
) {
    let only_moved = carried(
        thread_at('7', Some(version('4')), 1, "a"),
        version('5'),
        2,
        2,
        None,
    );
    let (mut stage, mut cx) = open_on_story(
        review_of_a(vec![round(1, None, false)], vec![only_moved]),
        history_of("5"),
        "a\nb\n",
        "x\na\nc\n",
        cx,
    );
    wait_for_files(&stage.pane, &mut cx);
    let notice = |stage: &Stage, cx: &mut VisualTestContext| {
        stage.pane.read_with(cx, |pane, _| pane.notice())
    };
    let first_row = bounds_of_row(2, &mut cx).center();
    cx.simulate_click(first_row, Modifiers::none());
    cx.update(|window, _| assert!(stage.focus.is_focused(window)));
    assert_eq!(notice(&stage, &mut cx), None);

    cx.simulate_keystrokes("w");
    let said = notice(&stage, &mut cx).expect("the notice says no thread moved here");
    assert!(said.contains("No thread"), "{said}");

    cx.simulate_keystrokes("shift-s");
    let said = notice(&stage, &mut cx).expect("the notice says no round has a state");
    assert!(said.contains("No round"), "{said}");
    assert_eq!(
        shown(&stage, &mut cx),
        ChangeShown::Comparison,
        "nothing was selected"
    );

    click("review-history-0", &mut cx);
    let (_, next) = stage.keeper.request(&mut cx);
    assert_eq!(
        next,
        Request::CompareVersions {
            workspace: id('1'),
            from: COMMIT.to_owned(),
            to: Some(version('5')),
        }
    );
}

#[gpui_kit::test]
fn a_round_with_no_recorded_state_of_the_worktree_to_count_to_says_so_and_selects_nothing(
    cx: &mut TestAppContext,
) {
    let history = History {
        workspace: id('1'),
        start: Some(COMMIT.to_owned()),
        entries: vec![HistoryEntry::Turn {
            turn: TurnId::new("a".repeat(32)),
            session: session_id('a'),
            state: None,
        }],
    };
    let (mut stage, mut cx) = open_on_story(
        review_of_a(vec![round(1, Some('5'), true)], Vec::new()),
        history,
        "a\nb\n",
        "a\nc\n",
        cx,
    );
    click("review-round-1-since", &mut cx);
    let said = stage
        .pane
        .read_with(&cx, |pane, _| pane.notice())
        .expect("the notice says there is nothing to count to");
    assert!(said.contains("No state"), "{said}");
    assert_eq!(shown(&stage, &mut cx), ChangeShown::Comparison);

    focus_the_rows(&stage, &mut cx);
    cx.simulate_keystrokes("shift-r");
    let (_, next) = stage.keeper.request(&mut cx);
    assert_eq!(
        next,
        Request::MarkReviewed {
            review: review_id()
        }
    );
}

#[gpui_kit::test]
fn a_round_whose_state_is_the_newest_recorded_shows_no_change_and_a_state_the_history_does_not_hold_is_counted_to_its_newest(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_on_story(
        review_of_a(vec![round(1, Some('7'), true)], Vec::new()),
        history_of("57"),
        "a\nb\n",
        "a\nc\n",
        cx,
    );
    click("review-round-1-since", &mut cx);
    let said = stage
        .pane
        .read_with(&cx, |pane, _| pane.notice())
        .expect("the notice says nothing was recorded since the send");
    assert!(said.contains("Nothing"), "{said}");
    assert_eq!(shown(&stage, &mut cx), ChangeShown::Comparison);

    focus_the_rows(&stage, &mut cx);
    cx.simulate_keystrokes("shift-s");
    let said = stage
        .pane
        .read_with(&cx, |pane, _| pane.notice())
        .expect("so does Since Round, with no round that has something since");
    assert!(said.contains("Nothing"), "{said}");
    assert_eq!(shown(&stage, &mut cx), ChangeShown::Comparison);

    click("review-history-0", &mut cx);
    let (_, next) = stage.keeper.request(&mut cx);
    assert_eq!(
        next,
        Request::CompareVersions {
            workspace: id('1'),
            from: COMMIT.to_owned(),
            to: Some(version('5')),
        }
    );

    tell_reviews(
        &mut stage,
        review_of_a(
            vec![round(1, Some('5'), false), round(2, Some('7'), true)],
            Vec::new(),
        ),
        &mut cx,
    );
    focus_the_rows(&stage, &mut cx);
    cx.simulate_keystrokes("shift-s");
    assert_eq!(
        shown(&stage, &mut cx),
        ChangeShown::Since {
            from: Some(version('5')),
            to: version('7'),
        }
    );

    tell_reviews(
        &mut stage,
        review_of_a(
            vec![round(1, Some('5'), false), round(2, Some('9'), true)],
            Vec::new(),
        ),
        &mut cx,
    );
    cx.simulate_keystrokes("shift-s");
    assert_eq!(
        shown(&stage, &mut cx),
        ChangeShown::Since {
            from: Some(version('9')),
            to: version('7'),
        }
    );
}

#[gpui_kit::test]
fn a_reply_opens_on_a_thread_that_moved_from_the_line_it_sits_on_now_and_not_from_the_line_it_was_written_on(
    cx: &mut TestAppContext,
) {
    let moved = carried(
        thread_at('7', Some(version('4')), 3, "c"),
        version('5'),
        5,
        5,
        None,
    );
    let (stage, mut cx) = open_on_story(
        review_of_a(vec![round(1, Some('5'), true)], vec![moved]),
        history_of("5"),
        "",
        "a\nb\nc\nd\ne\nf\n",
        cx,
    );
    let row_of_line = |line: u32, stage: &Stage, cx: &mut VisualTestContext| {
        stage
            .pane
            .read_with(cx, |pane, _| {
                pane.rows().iter().position(
                    |row| matches!(row, PaneRow::Diff { row, .. } if row.line(Side::New) == Some(line - 1)),
                )
            })
            .unwrap_or_else(|| panic!("no row shows new line {line}"))
    };
    let on_line = |line: u32, stage: &Stage, cx: &mut VisualTestContext| {
        let row = row_of_line(line, stage, cx);
        let at = bounds_of_row(row, cx).center();
        cx.simulate_click(at, Modifiers::none());
        cx.update(|window, _| assert!(stage.focus.is_focused(window)));
    };
    let the = thread_id('7');
    let row_five = row_of_line(5, &stage, &mut cx);
    assert_eq!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.rows()[row_five + 1].clone()),
        PaneRow::Thread {
            thread: the.clone()
        },
        "the pane draws the thread after line 5"
    );

    on_line(5, &stage, &mut cx);
    cx.simulate_keystrokes("o");
    let opened = stage
        .pane
        .read_with(&cx, |pane, _| pane.comment_box())
        .expect("a reply box opens on the line the thread sits on");
    assert_eq!(opened.thread, the);
    cx.simulate_keystrokes("escape");

    on_line(3, &stage, &mut cx);
    cx.simulate_keystrokes("o");
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.comment_box()),
        None
    );
    let said = stage
        .pane
        .read_with(&cx, |pane, _| pane.notice())
        .expect("the notice says there is no thread on the line");
    assert!(said.contains("no thread"), "{said}");
}

fn go_to_new_line(line: u32, stage: &Stage, cx: &mut VisualTestContext) {
    let row = stage
        .pane
        .read_with(cx, |pane, _| {
            pane.rows().iter().position(
                |row| matches!(row, PaneRow::Diff { row, .. } if row.line(Side::New) == Some(line - 1)),
            )
        })
        .unwrap_or_else(|| panic!("no row shows new line {line}"));
    let at = bounds_of_row(row, cx).center();
    cx.simulate_click(at, Modifiers::none());
    cx.update(|window, _| assert!(stage.focus.is_focused(window)));
}

#[gpui_kit::test]
fn edit_delete_resolve_and_attach_act_on_a_thread_that_moved_from_the_line_it_sits_on_now_and_do_nothing_from_the_line_it_was_written_on(
    cx: &mut TestAppContext,
) {
    let mut draft = comment('7');
    draft.round = None;
    let mut moved = carried(
        thread_at('7', Some(version('4')), 3, "c"),
        version('5'),
        5,
        5,
        None,
    );
    moved.comments = vec![draft];
    moved.state = ThreadState::Draft;
    let (mut stage, mut cx) = open_on_story(
        review_of_a(vec![round(1, Some('5'), true)], vec![moved]),
        history_of("5"),
        "",
        "a\nb\nc\nd\ne\nf\n",
        cx,
    );
    let the = thread_id('7');
    let notice = |stage: &Stage, cx: &mut VisualTestContext| {
        stage.pane.read_with(cx, |pane, _| pane.notice())
    };

    go_to_new_line(3, &stage, &mut cx);
    for (keys, words) in [
        ("i", "no draft"),
        ("d d", "no draft"),
        ("x", "no thread"),
        ("t", "no thread"),
    ] {
        cx.simulate_keystrokes(keys);
        assert_eq!(
            stage.pane.read_with(&cx, |pane, _| pane.comment_box()),
            None,
            "{keys} opened a box on the line the thread was written on"
        );
        let said = notice(&stage, &mut cx)
            .unwrap_or_else(|| panic!("{keys} on the line the thread was written on said nothing"));
        assert!(said.contains(words), "{keys}: {said}");
    }

    go_to_new_line(5, &stage, &mut cx);
    cx.simulate_keystrokes("i");
    let opened = stage
        .pane
        .read_with(&cx, |pane, _| pane.comment_box())
        .expect("the draft opens on the line the thread sits on");
    assert_eq!(
        (opened.thread, opened.comment),
        (the.clone(), comment('7').id)
    );
    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes("d d");
    let asked = stage.keeper.expect(
        Request::DeleteDraft {
            review: review_id(),
            comment: comment('7').id,
        },
        &mut cx,
    );
    stage.keeper.reply(asked, Ok(Reply::Done));
    cx.simulate_keystrokes("x");
    let asked = stage.keeper.expect(
        Request::ResolveThread {
            review: review_id(),
            thread: the.clone(),
        },
        &mut cx,
    );
    stage.keeper.reply(asked, Ok(Reply::Done));
    cx.simulate_keystrokes("t");
    let asked = stage.keeper.expect(
        Request::AttachReview {
            review: review_id(),
            thread: Some(the),
        },
        &mut cx,
    );
    stage
        .keeper
        .reply(asked, Ok(Reply::AttachedReview { start: Vec::new() }));
}

#[gpui_kit::test]
fn in_a_comparison_of_versions_a_moved_thread_is_acted_on_from_where_that_version_draws_it_and_from_no_line_in_a_third_version(
    cx: &mut TestAppContext,
) {
    let moved = carried(
        thread_at('7', Some(version('4')), 3, "c"),
        version('5'),
        5,
        5,
        None,
    );
    let (mut stage, mut cx) = open_on_story(
        review_of_a(vec![round(1, Some('4'), true)], vec![moved]),
        history_of("456"),
        "",
        "a\nb\nc\nd\ne\nf\n",
        cx,
    );
    let the = thread_id('7');
    let select =
        |stage: &mut Stage, entry: usize, from: String, to: String, cx: &mut VisualTestContext| {
            click(&format!("review-history-{entry}"), cx);
            compared(
                stage,
                from.clone(),
                Some(to.clone()),
                vec![file("story.txt", FileStatus::Modified)],
                cx,
            );
            read_story(stage, Some(from), Some(to), ("", "a\nb\nc\nd\ne\nf\n"), cx);
            wait_for_files(&stage.pane, cx);
        };
    let reply_from = |line: u32, stage: &Stage, cx: &mut VisualTestContext| {
        go_to_new_line(line, stage, cx);
        cx.simulate_keystrokes("o");
        let opened = stage.pane.read_with(cx, |pane, _| pane.comment_box());
        if opened.is_some() {
            cx.simulate_keystrokes("escape");
        } else {
            let said = stage
                .pane
                .read_with(cx, |pane, _| pane.notice())
                .unwrap_or_else(|| panic!("o on line {line} opened nothing and said nothing"));
            assert!(said.contains("no thread"), "line {line}: {said}");
        }
        opened.map(|opened| opened.thread)
    };
    let resolve_from = |line: u32, stage: &Stage, cx: &mut VisualTestContext| {
        go_to_new_line(line, stage, cx);
        cx.simulate_keystrokes("x");
        let said = stage
            .pane
            .read_with(cx, |pane, _| pane.notice())
            .unwrap_or_else(|| panic!("x on line {line} said nothing"));
        assert!(said.contains("no thread"), "line {line}: {said}");
    };

    select(&mut stage, 0, COMMIT.to_owned(), version('4'), &mut cx);
    assert_eq!(reply_from(3, &stage, &mut cx), Some(the.clone()));
    assert_eq!(reply_from(5, &stage, &mut cx), None);
    resolve_from(5, &stage, &mut cx);

    select(&mut stage, 2, version('5'), version('6'), &mut cx);
    assert!(stage.pane.read_with(&cx, |pane, _| {
        pane.rows().contains(&PaneRow::ThreadsElsewhere)
    }));
    assert_eq!(reply_from(3, &stage, &mut cx), None);
    assert_eq!(reply_from(5, &stage, &mut cx), None);
    resolve_from(3, &stage, &mut cx);
    resolve_from(5, &stage, &mut cx);

    select(&mut stage, 1, version('4'), version('5'), &mut cx);
    assert_eq!(reply_from(5, &stage, &mut cx), Some(the));
    assert_eq!(reply_from(3, &stage, &mut cx), None);
}

#[gpui_kit::test]
fn a_round_sent_in_a_turn_that_still_runs_is_not_counted_and_is_once_the_history_holds_that_turn_with_its_state(
    cx: &mut TestAppContext,
) {
    // Round 2 was sent in turn `7`, which the history lacks: its newest turn is `5`, older than the
    // state recorded at the send.
    let (mut stage, mut cx) = open_on_story(
        review_of_a(
            vec![round(1, Some('4'), true), round_in_turn(2, '6', '7')],
            Vec::new(),
        ),
        history_of("5"),
        "a\nb\n",
        "a\nc\n",
        cx,
    );
    assert!(is_drawn("review-round-2-since", &mut cx));

    focus_the_rows(&stage, &mut cx);
    cx.simulate_keystrokes("shift-s");
    let said = notice(&stage, &mut cx).expect("the notice says the turn still runs");
    assert!(
        said.contains("still runs") && said.contains("Round 2"),
        "{said}"
    );
    assert_eq!(shown(&stage, &mut cx), ChangeShown::Comparison);

    click("review-round-2-since", &mut cx);
    let said = notice(&stage, &mut cx).expect("the notice says the turn still runs");
    assert!(
        said.contains("still runs") && said.contains("Round 2"),
        "{said}"
    );
    assert_eq!(shown(&stage, &mut cx), ChangeShown::Comparison);

    cx.simulate_keystrokes("shift-s");
    assert_eq!(
        shown(&stage, &mut cx),
        ChangeShown::Since {
            from: Some(version('4')),
            to: version('5'),
        }
    );
    compared(
        &mut stage,
        version('4'),
        Some(version('5')),
        Vec::new(),
        &mut cx,
    );

    cx.simulate_keystrokes("shift-s");
    let said = notice(&stage, &mut cx).expect("the notice says the turn still runs");
    assert!(said.contains("Round 2"), "{said}");
    assert_eq!(
        shown(&stage, &mut cx),
        ChangeShown::Since {
            from: Some(version('4')),
            to: version('5'),
        }
    );
    cx.simulate_keystrokes("shift-s");
    assert_eq!(notice(&stage, &mut cx), None);
    assert_eq!(
        shown(&stage, &mut cx),
        ChangeShown::Since {
            from: Some(version('4')),
            to: version('5'),
        }
    );

    cx.simulate_keystrokes("shift-s");
    let said = notice(&stage, &mut cx).expect("the notice says the turn still runs");
    assert!(said.contains("Round 2"), "{said}");

    history_grows(&mut stage, history_of("57"), &mut cx);
    cx.simulate_keystrokes("shift-s");
    assert_eq!(notice(&stage, &mut cx), None);
    assert_eq!(
        shown(&stage, &mut cx),
        ChangeShown::Since {
            from: Some(version('6')),
            to: version('7'),
        }
    );
    compared(
        &mut stage,
        version('6'),
        Some(version('7')),
        Vec::new(),
        &mut cx,
    );

    click("review-history-0", &mut cx);
    compared(
        &mut stage,
        COMMIT.to_owned(),
        Some(version('5')),
        Vec::new(),
        &mut cx,
    );
    click("review-round-2-since", &mut cx);
    assert_eq!(notice(&stage, &mut cx), None);
    assert_eq!(
        shown(&stage, &mut cx),
        ChangeShown::Since {
            from: Some(version('6')),
            to: version('7'),
        }
    );
    compared(
        &mut stage,
        version('6'),
        Some(version('7')),
        Vec::new(),
        &mut cx,
    );
    click("review-history-0", &mut cx);
    compared(
        &mut stage,
        COMMIT.to_owned(),
        Some(version('5')),
        Vec::new(),
        &mut cx,
    );
    focus_the_rows(&stage, &mut cx);
    cx.simulate_keystrokes("shift-s");
    assert_eq!(notice(&stage, &mut cx), None);
    assert_eq!(
        shown(&stage, &mut cx),
        ChangeShown::Since {
            from: Some(version('6')),
            to: version('7'),
        }
    );
    compared(
        &mut stage,
        version('6'),
        Some(version('7')),
        Vec::new(),
        &mut cx,
    );
}

#[gpui_kit::test]
fn a_turn_the_history_holds_with_no_state_is_a_turn_that_still_runs_and_so_is_one_in_a_history_with_no_entry(
    cx: &mut TestAppContext,
) {
    let mut history = history_of("5");
    history.entries.push(HistoryEntry::Turn {
        turn: TurnId::new("7".repeat(32)),
        session: session_id('a'),
        state: None,
    });
    let (mut stage, mut cx) = open_on_story(
        review_of_a(vec![round_in_turn(1, '6', '7')], Vec::new()),
        history,
        "a\nb\n",
        "a\nc\n",
        cx,
    );
    click("review-round-1-since", &mut cx);
    let said = notice(&stage, &mut cx).expect("the notice says the turn still runs");
    assert!(said.contains("still runs"), "{said}");
    assert_eq!(shown(&stage, &mut cx), ChangeShown::Comparison);

    // An empty history does not hold the turn either: the notice still says the turn runs,
    // not that no state is recorded.
    history_grows(&mut stage, history_of(""), &mut cx);
    focus_the_rows(&stage, &mut cx);
    cx.simulate_keystrokes("shift-s");
    let said = notice(&stage, &mut cx).expect("the notice says the turn still runs");
    assert!(said.contains("still runs"), "{said}");
    assert_eq!(shown(&stage, &mut cx), ChangeShown::Comparison);

    history_grows(&mut stage, history_of("5"), &mut cx);
    click("review-history-0", &mut cx);
    let (_, next) = stage.keeper.request(&mut cx);
    assert_eq!(
        next,
        Request::CompareVersions {
            workspace: id('1'),
            from: COMMIT.to_owned(),
            to: Some(version('5')),
        }
    );
}

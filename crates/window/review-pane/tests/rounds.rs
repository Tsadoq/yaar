//! Component test: the rounds of a review and the threads that moved with the code, in the
//! Review pane on GPUI's test platform, with no window on any screen. Keys, clicks and the
//! wheel go in; the pane is read back through the model it shows. The test plays the Keeper
//! on the other end of a socket pair as the Keeper of slice 6 plays it: what the Keeper
//! works out (a round's state and its `edited`, a thread's placement and notice) is set as
//! the data the Link carries, and a new answer to `GetReviews` comes after
//! `Fact::ReviewsChanged`.
//!
//! A negative clause ("not drawn", "sends nothing") is read after a barrier: something the
//! test causes after it, through the same path, whose arrival shows that everything before
//! it arrived.

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

/// A state or a commit: this digit forty times.
fn version(digit: char) -> String {
    digit.to_string().repeat(40)
}

fn review_id() -> ReviewId {
    ReviewId::new("1".repeat(32))
}

fn story() -> PathBuf {
    PathBuf::from("story.txt")
}

/// A turn of session `a` that ended at the state `digit` forty times.
fn turn(digit: char) -> HistoryEntry {
    HistoryEntry::Turn {
        turn: TurnId::new(digit.to_string().repeat(32)),
        session: session_id('a'),
        state: Some(version(digit)),
    }
}

/// The history of the tests: the turns that ended at these states, in order, counted from
/// the commit the changes are counted from.
fn history_of(states: &str) -> History {
    History {
        workspace: id('1'),
        start: Some(COMMIT.to_owned()),
        entries: states.chars().map(turn).collect(),
    }
}

/// A round sent with the review, delivered to the agent.
fn round(number: u32, state: Option<char>, edited: bool) -> Round {
    Round {
        number,
        state: state.map(version),
        sent_by: SentBy::Delivered,
        edited,
        turn: None,
    }
}

/// A round sent in the turn of the digit `turn` (see [`turn`]), which the history may not
/// hold yet.
fn round_in_turn(number: u32, state: char, turn: char) -> Round {
    Round {
        turn: Some(TurnId::new(turn.to_string().repeat(32))),
        ..round(number, Some(state), false)
    }
}

/// The review of session `a` with these rounds.
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

/// A thread of round one, written on the new side of `story.txt` at line `line` in the
/// version `tree`, on the text `text`; it has not moved.
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

/// The thread, carried by the Keeper to the lines `first` to `last` of `story.txt` in the
/// version `tree`, with what became of the code it was written on.
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

/// A window with the pane in it, that shows the changes of workspace `1` against its base
/// (the file `story.txt` from `old` to `new`), with the review of session `a` and then the
/// history, which is set after the changes are answered: a pane whose changes are not yet
/// here shows the last recorded turn instead of them.
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

/// Whether the element with this selector was drawn in a frame made now: what was drawn
/// before the last change is not what is read.
fn is_drawn(selector: &str, cx: &mut VisualTestContext) -> bool {
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    drawn(selector, cx).is_some()
}

/// Clicks the middle of the element with this selector.
fn click(selector: &str, cx: &mut VisualTestContext) {
    assert!(is_drawn(selector, cx), "{selector} is not drawn");
    let at = drawn(selector, cx).expect("it was drawn").center();
    cx.simulate_click(at, Modifiers::none());
}

/// The Keeper tells that the reviews changed, to these, and the test waits until the pane
/// holds as many rounds and threads.
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

/// The Keeper's history is now this one, and the pane has read it: no turn end is told, so
/// that nothing is selected by it.
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

/// Answers the read of the two versions of `story.txt` the pane asks for.
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

/// Puts the focus on the rows of the pane, where the keys of the pane work.
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

    // After a send the pane shows the round as sent, with the state recorded at the send.
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

    // Until the agent's first edit: the answer to the next question for the reviews has
    // the round edited.
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

    // What changed since the comments of the round were sent, by a click on the round's
    // control: from the state recorded at the send to the newest state of the history.
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

    // The same by the key, from another selection: the first turn of the tree.
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

    // A second round with no state yet: it shows as sent, and so does the first, whose agent
    // has made no edit since either send. The control of the second is not drawn. Both are
    // read in the frame that draws round two.
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

    // Until a new answer gives it a state, which draws the control.
    tell_reviews(
        &mut stage,
        review_of_a(
            vec![round(1, Some('5'), false), round(2, Some('6'), false)],
            Vec::new(),
        ),
        &mut cx,
    );
    assert!(is_drawn("review-round-2-since", &mut cx));

    // The agent's first edit after the second send is an edit after the first send too:
    // both rounds show as answered.
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

    // Since Round: the first press shows the newest round's change, the next the round
    // before, and after the first round the newest again.
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
    // The new side has lines 3 and 4 rewritten; the thread was written on line 3 of an
    // earlier version, where it read "old line", and the Keeper carried it to both.
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

    // The thread follows the row of new line 4 (counted from 1), the last of the lines it
    // was carried to, and not the row of the line it was written on.
    let line_four = index_of_new_line(3, &stage, &mut cx);
    assert_eq!(
        all_rows(&stage, &mut cx)[line_four + 1],
        PaneRow::Thread {
            thread: the.clone()
        }
    );

    // It says that the code changed, and the other that the code was deleted.
    assert!(is_drawn(
        &format!("review-thread-notice-{the}:changed"),
        &mut cx
    ));
    assert!(drawn(&format!("review-thread-notice-{other}:deleted"), &mut cx).is_some());
    assert!(drawn(&format!("review-thread-notice-{the}:deleted"), &mut cx).is_none());

    // The reviewer on line 4: `w` shows the line the comment was written on, below the
    // thread's row; `w` again hides it.
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

    // A click on the thread's control shows it, and a click on it again hides it.
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

    // The text a row draws is the text the pane gives for it, all of it: a comment written on
    // two lines shows both, on the one row. The test platform draws no glyph that a test can
    // read, but it lays the text out in a monospace font, so the width of what is drawn
    // tells how many characters it holds: both texts are shown, and the width of each is the
    // same width for each character of the text the pane says it draws.
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
    // Written in the version `1`, carried to the version `3`, a rewrite.
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
    // Files on disk, which the Keeper carries every thread toward: on its lines.
    assert!(!elsewhere(&stage, &mut cx));
    assert!(is_drawn(&notice, &mut cx));

    // The turn that ended at the version `2`: neither where it was written nor where it
    // was carried to, so the thread is under Threads elsewhere, with no notice drawn.
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

    // The turn that ended at the version it was carried to: on its lines again, with its
    // notice, and no longer elsewhere.
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

    // The reviewer asks for the line it was written on, where it was carried to.
    let original = format!("review-thread-original-text-{the}");
    click(&format!("review-thread-original-{the}"), &mut cx);
    assert!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.shows_original(&the))
    );
    assert!(is_drawn(&original, &mut cx));

    // The turn that ended at the version it was written in: on the line it was written on,
    // as written, so no notice of code that changed, and no original to show: the line is
    // the one it was written on.
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
    // A thread that only moved: it carries no notice, so there is no original to show.
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

    // Barrier: a request the pane sends after both, through the same connection, is the
    // next the Keeper hears, so neither key sent anything before it.
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
    // The one turn of the history has no state: it was not recorded, or is no longer kept.
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

    // Barrier: the next request of the Window after the click is the one the reviewer's
    // next action sends, so the click sent none.
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
    // The one round was sent at the state the history ends with: nothing was recorded
    // since, and an empty change is no change to show.
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

    // Barrier: a request the pane sends after both is the next the Keeper hears, so
    // neither sent any before it.
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

    // With an older round beside it, Since Round leaves that round out and shows the one
    // that has something since.
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

    // A round sent at a state that is no entry of the history, as a state recorded at a
    // send is not: counted to the newest entry.
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
    // Written on line 3, carried by the Keeper to line 5, where the pane draws it.
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

    // On line 5, where the thread is drawn, a reply opens in it.
    on_line(5, &stage, &mut cx);
    cx.simulate_keystrokes("o");
    let opened = stage
        .pane
        .read_with(&cx, |pane, _| pane.comment_box())
        .expect("a reply box opens on the line the thread sits on");
    assert_eq!(opened.thread, the);
    cx.simulate_keystrokes("escape");

    // On line 3, which is other code now, there is no thread to reply in.
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

/// Stands the reviewer on the row that shows new line `line` (counted from 1) of
/// `story.txt`, by a click, as a reviewer does.
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
    // A draft written on line 3, carried by the Keeper to line 5, where the pane draws it.
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

    // On line 3, which is other code now, each key finds nothing and sends nothing.
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

    // On line 5, where it is drawn, edit opens the draft, and delete, resolve and attach ask
    // the Keeper, each the first request the Keeper hears since the keys above, which sent
    // none.
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
    // Written on line 3 in the version `4`, carried to line 5 in the version `5`.
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
    // What the key does on this line: the thread it opens a reply box in, if any.
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

    // The new side is the version the thread was written in: the pane draws it on the
    // line it was written on, and that is where a key finds it, not on the line `now` says.
    select(&mut stage, 0, COMMIT.to_owned(), version('4'), &mut cx);
    assert_eq!(reply_from(3, &stage, &mut cx), Some(the.clone()));
    assert_eq!(reply_from(5, &stage, &mut cx), None);
    resolve_from(5, &stage, &mut cx);

    // The new side is a third version, which the thread was neither written in nor carried
    // to: it is under Threads elsewhere, and no key finds it from either line.
    select(&mut stage, 2, version('5'), version('6'), &mut cx);
    assert!(stage.pane.read_with(&cx, |pane, _| {
        pane.rows().contains(&PaneRow::ThreadsElsewhere)
    }));
    assert_eq!(reply_from(3, &stage, &mut cx), None);
    assert_eq!(reply_from(5, &stage, &mut cx), None);
    resolve_from(3, &stage, &mut cx);
    resolve_from(5, &stage, &mut cx);

    // The new side is the version it was carried to: on the line `now` says, and not on
    // the one it was written on.
    select(&mut stage, 1, version('4'), version('5'), &mut cx);
    assert_eq!(reply_from(5, &stage, &mut cx), Some(the));
    assert_eq!(reply_from(3, &stage, &mut cx), None);
}

#[gpui_kit::test]
fn a_round_sent_in_a_turn_that_still_runs_is_not_counted_and_is_once_the_history_holds_that_turn_with_its_state(
    cx: &mut TestAppContext,
) {
    // Round 1 has no turn, as a copy has none: counted as ever. Round 2 was sent in the
    // turn `7`, which the history does not hold: its newest turn is `5`, older than the
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

    // Since Round, for the newest round, which is the first it goes to, says the turn
    // runs and selects nothing.
    focus_the_rows(&stage, &mut cx);
    cx.simulate_keystrokes("shift-s");
    let said = notice(&stage, &mut cx).expect("the notice says the turn still runs");
    assert!(
        said.contains("still runs") && said.contains("Round 2"),
        "{said}"
    );
    assert_eq!(shown(&stage, &mut cx), ChangeShown::Comparison);

    // So does the control of the round, and it selects nothing either.
    click("review-round-2-since", &mut cx);
    let said = notice(&stage, &mut cx).expect("the notice says the turn still runs");
    assert!(
        said.contains("still runs") && said.contains("Round 2"),
        "{said}"
    );
    assert_eq!(shown(&stage, &mut cx), ChangeShown::Comparison);

    // The press after the notice passes to the round before, which has no turn and is
    // counted to the newest turn of the history, as before. This is the first request the
    // Keeper hears from the pane since the history: the press and the click sent none.
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

    // After the round before, the newest again: the turn still runs. The notice then
    // belongs to round 2, so the press after it passes on to the round before once more
    // and does not stay on round 2.
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

    // The turn ends: the history holds it, with its state. The round is counted to that
    // state, which is newer than the round's own, as RP-38 counts a round, and the
    // control and the key ask for it.
    history_grows(&mut stage, history_of("57"), &mut cx);
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
fn a_turn_the_history_holds_with_no_state_is_a_turn_that_still_runs_and_so_is_one_with_no_history_at_all(
    cx: &mut TestAppContext,
) {
    // The turn `7` is an entry of the history, but with no state recorded: not an entry
    // with a state, so the round is not counted.
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

    // No history at all: the turn still runs is what is true, not that no state is
    // recorded.
    history_grows(&mut stage, history_of(""), &mut cx);
    focus_the_rows(&stage, &mut cx);
    cx.simulate_keystrokes("shift-s");
    let said = notice(&stage, &mut cx).expect("the notice says the turn still runs");
    assert!(said.contains("still runs"), "{said}");
    assert_eq!(shown(&stage, &mut cx), ChangeShown::Comparison);

    // Barrier: a request the pane sends after all of these is the next the Keeper hears,
    // so none of the presses and clicks sent one before it. The history has an entry to
    // click again.
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

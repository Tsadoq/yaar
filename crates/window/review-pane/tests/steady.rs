//! Component test: Changes holds still, on GPUI's test platform. Keys, clicks and the wheel
//! go in; the pane is read back through the model it shows. The test plays the Keeper on the
//! other end of a socket pair: it tells the facts the Keeper tells, and, since the Keeper of
//! this slice does not yet send a reply or a moved thread, it plays them as the Keeper will:
//! a new answer to `GetReviews`, after `Fact::ReviewsChanged`.
//!
//! A negative clause ("does not move") is read after a barrier: something the test causes
//! after it, through the same path, whose arrival shows that everything before it arrived.

mod common;

use std::path::PathBuf;

use gpui_kit::{Modifiers, ScrollDelta, ScrollEvent, TestAppContext, VisualTestContext, point, px};
use yaar_link::{
    Anchor, Author, Comment, CommentId, Fact, FileStatus, Lines, ReviewId, SessionState,
    Side as LinkSide, Step, Thread, ThreadId, WorkspaceReviews,
};
use yaar_review_pane::{PaneRow, Position, View};

use common::{
    Stage, agents_of, bounds_of_row, file, id, open_staged, review_with, reviews_of, running_turn,
    session, session_id, since_main, step_fact, texts, turn_id, wait_for_files, wait_until,
    working_on, workspace,
};

/// The lines of the file the pane shows: all of them added, so all of them changed.
const LINES: usize = 40;

fn review_id() -> ReviewId {
    ReviewId::new("1".repeat(32))
}

fn lines_of_the_file() -> String {
    (0..LINES).map(|line| format!("line {line}\n")).collect()
}

/// A comment of the thread, sent in round one.
fn comment(digit: char, author: Author) -> Comment {
    Comment {
        id: CommentId::new(digit.to_string().repeat(32)),
        author,
        text: format!("comment {digit}"),
        round: Some(1),
    }
}

/// The comments of a long thread, `count` of them, each sent in round one.
fn comments(count: usize) -> Vec<Comment> {
    comments_of('0', count)
}

/// As [`comments`], with ids that start with `digit`, so that two threads share none.
fn comments_of(digit: char, count: usize) -> Vec<Comment> {
    (0..count)
        .map(|number| Comment {
            id: CommentId::new(format!("{digit}{number:031}")),
            author: Author::Reviewer,
            text: format!("comment {number}"),
            round: Some(1),
        })
        .collect()
}

/// The thread of the review, on line `line` (counted from 1) of `long.txt`, new side.
fn thread_on(line: u32, comments: Vec<Comment>) -> Thread {
    thread_with('7', line, comments)
}

/// As [`thread_on`], for the thread whose id is `digit` thirty-two times.
fn thread_with(digit: char, line: u32, comments: Vec<Comment>) -> Thread {
    Thread {
        id: ThreadId::new(digit.to_string().repeat(32)),
        anchor: Anchor {
            tree: None,
            path: PathBuf::from("long.txt"),
            side: LinkSide::New,
            lines: Lines {
                first: line,
                last: line,
            },
            text: format!("line {}", line - 1),
        },
        resolved: false,
        addressed: false,
        comments,
    }
}

/// The review of session `a`, with this thread.
fn review_of_a(thread: Thread) -> WorkspaceReviews {
    review_of_a_with(vec![thread])
}

/// The review of session `a`, with these threads.
fn review_of_a_with(threads: Vec<Thread>) -> WorkspaceReviews {
    let mut review = review_with(review_id(), Some(session_id('a')), None);
    review.threads = threads;
    reviews_of(vec![review])
}

/// What the reviewer reads: the view on screen, the first row in view, where it is, and the
/// line they are on.
#[derive(Debug, PartialEq)]
struct Reading {
    view: View,
    first: PaneRow,
    start: usize,
    position: Option<Position>,
}

/// Reads the pane on a layout made now: a change of the scroll position that notifies
/// nothing is seen as well.
fn reading(stage: &Stage, cx: &mut VisualTestContext) -> Reading {
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    stage.pane.read_with(cx, |pane, _| {
        let start = pane.rows_in_view().start;
        Reading {
            view: pane.view(),
            first: pane.rows()[start].clone(),
            start,
            position: pane.position().cloned(),
        }
    })
}

/// The first row in view that is not a thread or a comment: the diff the reviewer reads,
/// with how far below the top of the view it lies.
fn diff_in_view(stage: &Stage, cx: &mut VisualTestContext) -> (PaneRow, usize) {
    let held = reading(stage, cx);
    stage.pane.read_with(cx, |pane, _| {
        let (index, row) = pane
            .rows()
            .iter()
            .enumerate()
            .skip(held.start)
            .find(|(_, row)| !matches!(row, PaneRow::Thread { .. } | PaneRow::Comment { .. }))
            .expect("a row of the diff is in view");
        (row.clone(), index - held.start)
    })
}

/// How far below the top of the view `row` lies now.
fn below_the_top(row: &PaneRow, stage: &Stage, cx: &mut VisualTestContext) -> isize {
    let start = reading(stage, cx).start;
    stage.pane.read_with(cx, |pane, _| {
        let index = pane
            .rows()
            .iter()
            .position(|held| held == row)
            .expect("the row is still among the rows");
        index as isize - start as isize
    })
}

/// The index of the row of the thread, among the rows of the pane.
fn thread_row(stage: &Stage, cx: &mut VisualTestContext) -> usize {
    rows_of(stage, cx)
        .iter()
        .position(|row| matches!(row, PaneRow::Thread { .. }))
        .expect("the thread has a row")
}

fn rows_of(stage: &Stage, cx: &mut VisualTestContext) -> Vec<PaneRow> {
    stage.pane.read_with(cx, |pane, _| pane.rows().to_vec())
}

/// Turns the wheel over the rows, `by` pixels (negative: down the list).
fn wheel(stage: &Stage, by: f32, cx: &mut VisualTestContext) {
    let in_view = stage.pane.read_with(cx, |pane, _| pane.rows_in_view());
    let over_the_rows = bounds_of_row(in_view.start + 2, cx).center();
    cx.simulate_event(ScrollEvent {
        position: over_the_rows,
        delta: ScrollDelta::Pixels(point(px(0.), px(by))),
        ..ScrollEvent::default()
    });
    cx.run_until_parked();
}

/// Turns the wheel, a row at a time, until `row` is the first row in view.
fn scroll_until_first(stage: &Stage, row: usize, cx: &mut VisualTestContext) {
    loop {
        let start = reading(stage, cx).start;
        assert!(start <= row, "the view is past row {row}, at {start}");
        if start == row {
            return;
        }
        let height = f32::from(bounds_of_row(start, cx).size.height);
        wheel(stage, -height, cx);
    }
}

/// Shows the changes of the one long file, and waits until the pane holds its rows, and
/// the thread's among them when the review has one.
fn show_long_file(stage: &mut Stage, with_thread: bool, cx: &mut VisualTestContext) {
    let changes = since_main(Vec::new(), vec![file("long.txt", FileStatus::Added)]);
    stage
        .keeper
        .show(&changes, |_| texts("", &lines_of_the_file()), cx);
    wait_for_files(&stage.pane, cx);
    wait_until(cx, "the review of the session", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.review().is_some()
                && pane
                    .rows()
                    .iter()
                    .any(|row| matches!(row, PaneRow::Thread { .. }))
                    == with_thread
        })
    });
}

/// The reviewer clicks into the rows, walks down them by keys and turns the wheel, so that
/// the rows in view start below `above`.
fn read_down_the_file(stage: &Stage, above: usize, cx: &mut VisualTestContext) {
    let row = bounds_of_row(2, cx).center();
    cx.simulate_click(row, Modifiers::none());
    cx.update(|window, _| assert!(stage.focus.is_focused(window)));
    for _ in 0..30 {
        cx.simulate_keystrokes("j");
    }
    wheel(stage, -40., cx);
    let start = reading(stage, cx).start;
    assert!(
        start > above,
        "the rows in view start at {start}, below the rows above the view"
    );
}

/// The Keeper answers the next `GetReviews` with the thread, and tells that the reviews
/// changed; returns when the pane holds a thread of `comments` comments.
fn tell_reviews(stage: &mut Stage, thread: Thread, comments: usize, cx: &mut VisualTestContext) {
    stage.keeper.set_reviews(review_of_a(thread));
    stage
        .keeper
        .tell(Fact::ReviewsChanged { workspace: id('1') });
    wait_until(cx, "the thread with its comments", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.reviews().is_some_and(|reviews| {
                reviews.reviews[0]
                    .threads
                    .first()
                    .is_some_and(|thread| thread.comments.len() == comments)
            })
        })
    });
}

/// As [`tell_reviews`], for a thread that moved to `line`.
fn tell_moved(stage: &mut Stage, thread: Thread, line: u32, cx: &mut VisualTestContext) {
    stage.keeper.set_reviews(review_of_a(thread));
    stage
        .keeper
        .tell(Fact::ReviewsChanged { workspace: id('1') });
    wait_until(cx, "the thread on its new line", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.reviews().is_some_and(|reviews| {
                reviews.reviews[0]
                    .threads
                    .first()
                    .is_some_and(|thread| thread.anchor.lines.first == line)
            })
        })
    });
}

#[gpui_kit::test]
fn no_fact_of_the_keeper_moves_what_changes_shows_a_step_a_turn_end_a_reply_or_a_thread_that_moved(
    cx: &mut TestAppContext,
) {
    let turn = turn_id('t');
    let running = working_on(
        session('a', "Claude Code 1", SessionState::Works, Some(review_id())),
        turn.clone(),
    );
    // A turn the pane meets running at the connect: that shows no Flow.
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature")],
        |keeper| {
            keeper.set_agents(vec![agents_of(vec![running.clone()])]);
            keeper.set_turn(running_turn(
                turn.clone(),
                session_id('a'),
                1,
                Some(&"5".repeat(40)),
                Vec::new(),
            ));
            // The thread is on line 3, above every row the reviewer scrolls to.
            keeper.set_reviews(review_of_a(thread_on(
                3,
                vec![comment('1', Author::Reviewer)],
            )));
        },
        cx,
    );
    show_long_file(&mut stage, true, &mut cx);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.view()),
        View::Changes
    );

    // The wheel and the keys put the rows in view past the thread, and the reviewer on a
    // line; `shift-t` keeps the selection at a turn end. They paused follow while the turn
    // ran, which the two presses of `shift-f` end.
    let thread = thread_row(&stage, &mut cx);
    read_down_the_file(&stage, thread + 1, &mut cx);
    cx.simulate_keystrokes("shift-t");
    cx.simulate_keystrokes("shift-f shift-f");
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.following()),
        yaar_review_pane::Following::On
    );
    let held = reading(&stage, &mut cx);
    assert!(held.position.is_some(), "the reviewer is on a line");

    // A step. The steps and then a question for the reviews, which the Keeper hears after
    // the steps and answers with the reviews as they were.
    for (number, step) in [
        (
            1,
            Step::Read {
                path: "long.txt".into(),
            },
        ),
        (
            2,
            Step::Write {
                path: "long.txt".into(),
            },
        ),
        (
            3,
            Step::Command {
                command: "make".to_owned(),
            },
        ),
    ] {
        stage
            .keeper
            .tell(step_fact(session_id('a'), turn.clone(), number, step));
    }
    let asked = stage.keeper.reviews_asked();
    stage
        .keeper
        .tell(Fact::ReviewsChanged { workspace: id('1') });
    wait_until(&mut cx, "the reviews to be asked for again", |_| {
        stage.keeper.reviews_asked() > asked
    });
    assert_eq!(reading(&stage, &mut cx), held, "after the steps");

    // A turn end that leaves the selection as it is: the offer is there, and nothing else.
    stage.keeper.tell(Fact::TurnEnded {
        workspace: id('1'),
        session: session_id('a'),
        turn: turn.clone(),
        state: Some("6".repeat(40)),
        since: None,
    });
    wait_until(&mut cx, "the offer at the turn end", |cx| {
        stage.pane.read_with(cx, |pane, _| pane.offer().is_some())
    });
    assert_eq!(reading(&stage, &mut cx), held, "after the turn end");

    // A reply: one more comment in the thread, above the rows in view. The row that was
    // first in view is first again, one row further down the list.
    tell_reviews(
        &mut stage,
        thread_on(
            3,
            vec![comment('1', Author::Reviewer), comment('2', Author::Agent)],
        ),
        2,
        &mut cx,
    );
    let after_reply = reading(&stage, &mut cx);
    assert_eq!(after_reply.view, View::Changes);
    assert_eq!(after_reply.first, held.first);
    assert_eq!(after_reply.start, held.start + 1, "a row came above it");
    assert_eq!(after_reply.position, held.position);

    // A thread that moved from line 3 to line 4, still above the rows in view.
    let before = thread_row(&stage, &mut cx);
    tell_moved(
        &mut stage,
        thread_on(
            4,
            vec![comment('1', Author::Reviewer), comment('2', Author::Agent)],
        ),
        4,
        &mut cx,
    );
    assert_eq!(
        thread_row(&stage, &mut cx),
        before + 1,
        "its rows moved down"
    );
    assert_eq!(reading(&stage, &mut cx), after_reply);

    // A thread that moved from above the rows in view to below them: its three rows leave
    // from above, and the row that was first in view is first again, three rows up the
    // list.
    tell_moved(
        &mut stage,
        thread_on(
            LINES as u32 - 1,
            vec![comment('1', Author::Reviewer), comment('2', Author::Agent)],
        ),
        LINES as u32 - 1,
        &mut cx,
    );
    let below = thread_row(&stage, &mut cx);
    let after_move = reading(&stage, &mut cx);
    let end = stage.pane.read_with(&cx, |pane, _| pane.rows_in_view().end);
    assert!(
        below >= end,
        "the thread's row {below} lies below the rows in view"
    );
    assert_eq!(after_move.view, View::Changes);
    assert_eq!(after_move.first, held.first);
    assert_eq!(
        after_move.start,
        held.start - 2,
        "three rows left, one came"
    );
    assert_eq!(after_move.position, held.position);

    // A reply in that thread, below the first row in view: no row came above it, and the
    // view stays where it was.
    tell_reviews(
        &mut stage,
        thread_on(
            LINES as u32 - 1,
            vec![
                comment('1', Author::Reviewer),
                comment('2', Author::Agent),
                comment('3', Author::Reviewer),
            ],
        ),
        3,
        &mut cx,
    );
    assert_eq!(reading(&stage, &mut cx), after_move, "after a reply below");
}

#[gpui_kit::test]
fn a_thread_deleted_in_another_window_takes_its_rows_from_above_the_view_and_the_view_goes_up_by_as_many(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature")],
        |keeper| {
            keeper.set_agents(vec![agents_of(vec![session(
                'a',
                "Claude Code 1",
                SessionState::Idle,
                Some(review_id()),
            )])]);
            keeper.set_reviews(review_of_a(thread_on(
                3,
                vec![comment('1', Author::Reviewer), comment('2', Author::Agent)],
            )));
        },
        cx,
    );
    show_long_file(&mut stage, true, &mut cx);
    let thread = thread_row(&stage, &mut cx);
    read_down_the_file(&stage, thread + 2, &mut cx);
    let held = reading(&stage, &mut cx);

    stage.keeper.set_reviews(reviews_of(vec![review_with(
        review_id(),
        Some(session_id('a')),
        None,
    )]));
    stage
        .keeper
        .tell(Fact::ReviewsChanged { workspace: id('1') });
    wait_until(&mut cx, "the thread to go", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            !pane
                .rows()
                .iter()
                .any(|row| matches!(row, PaneRow::Thread { .. }))
        })
    });

    let after = reading(&stage, &mut cx);
    assert_eq!(after.first, held.first);
    assert_eq!(
        after.start,
        held.start - 3,
        "a thread and two comments left"
    );
    assert_eq!(after.position, held.position);
}

#[gpui_kit::test]
fn another_review_shown_when_a_session_runs_brings_its_thread_above_the_view_and_the_view_goes_down_by_as_many(
    cx: &mut TestAppContext,
) {
    let (ra, rb) = (review_id(), ReviewId::new("2".repeat(32)));
    // Session `a` is not running and `b` works, so the review of `b` is shown; the review of
    // `a`, which holds the thread, is not, and no review is of no agent.
    let sessions = |a: SessionState| {
        agents_of(vec![
            session('a', "Claude Code 1", a, Some(ra.clone())),
            session('b', "Claude Code 2", SessionState::Works, Some(rb.clone())),
        ])
    };
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature")],
        |keeper| {
            keeper.set_agents(vec![sessions(SessionState::NotRunning)]);
            let mut first = review_with(ra.clone(), Some(session_id('a')), None);
            first
                .threads
                .push(thread_on(3, vec![comment('1', Author::Reviewer)]));
            keeper.set_reviews(reviews_of(vec![
                first,
                review_with(rb.clone(), Some(session_id('b')), None),
            ]));
        },
        cx,
    );
    show_long_file(&mut stage, false, &mut cx);
    // The thread of `a` would sit on rows 5 and 6; the rows in view start below them.
    read_down_the_file(&stage, 6, &mut cx);
    let held = reading(&stage, &mut cx);

    // `a` runs too, and is the first that does: its review is shown, with its thread.
    stage.keeper.tell_agents(sessions(SessionState::Idle));
    wait_until(&mut cx, "the review of the first session shown", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.review().map(|review| review.id.clone()) == Some(ra.clone())
        })
    });
    assert_eq!(thread_row(&stage, &mut cx), 5);

    let after = reading(&stage, &mut cx);
    assert_eq!(after.first, held.first);
    assert_eq!(after.start, held.start + 2, "a thread and its comment came");
    assert_eq!(after.position, held.position);
}

/// Shows the long file with these threads in the review of session `a`, which is idle.
fn stage_with_threads(threads: Vec<Thread>, cx: &mut TestAppContext) -> (Stage, VisualTestContext) {
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature")],
        |keeper| {
            keeper.set_agents(vec![agents_of(vec![session(
                'a',
                "Claude Code 1",
                SessionState::Idle,
                Some(review_id()),
            )])]);
            keeper.set_reviews(review_of_a_with(threads));
        },
        cx,
    );
    show_long_file(&mut stage, true, &mut cx);
    (stage, cx)
}

/// As [`stage_with_threads`] for one thread of `comments` on line `line`, with the wheel
/// turned until the first row in view is `after_the_thread` rows below the thread's own.
fn stage_with_thread_in_view(
    line: u32,
    comments: Vec<Comment>,
    after_the_thread: usize,
    cx: &mut TestAppContext,
) -> (Stage, VisualTestContext) {
    let (stage, mut cx) = stage_with_threads(vec![thread_on(line, comments)], cx);
    let thread = thread_row(&stage, &mut cx);
    scroll_until_first(&stage, thread + after_the_thread, &mut cx);
    (stage, cx)
}

/// The Keeper tells that the reviews changed, to a review with `threads`, and the test
/// waits until the pane holds as many.
fn tell_threads(stage: &mut Stage, threads: Vec<Thread>, cx: &mut VisualTestContext) {
    let count = threads.len();
    stage.keeper.set_reviews(review_of_a_with(threads));
    stage
        .keeper
        .tell(Fact::ReviewsChanged { workspace: id('1') });
    wait_until(cx, "the threads of the review", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.reviews()
                .is_some_and(|reviews| reviews.reviews[0].threads.len() == count)
        })
    });
}

#[gpui_kit::test]
fn a_thread_that_moves_while_its_row_is_first_in_view_leaves_the_diff_where_it_is_on_screen(
    cx: &mut TestAppContext,
) {
    let comments = vec![comment('1', Author::Reviewer)];
    let (mut stage, mut cx) = stage_with_thread_in_view(20, comments.clone(), 0, cx);
    let held = reading(&stage, &mut cx);
    assert!(
        matches!(held.first, PaneRow::Thread { .. }),
        "the thread is first in view"
    );
    let (diff, offset) = diff_in_view(&stage, &mut cx);
    assert_eq!(offset, 2, "the diff starts under the thread");
    let offset = offset as isize;

    // The thread moves ten lines down: its two rows leave from above the diff the reviewer
    // reads, which stays where it is on screen. The view does not follow the thread.
    tell_moved(&mut stage, thread_on(30, comments), 30, &mut cx);
    assert_eq!(below_the_top(&diff, &stage, &mut cx), offset);
    let after = reading(&stage, &mut cx);
    assert_eq!(after.view, View::Changes);
    assert_eq!(after.start, held.start - 2, "the thread's two rows left");
    assert_eq!(after.position, held.position);
}

#[gpui_kit::test]
fn a_thread_deleted_while_its_row_is_first_in_view_leaves_the_diff_where_it_is_on_screen(
    cx: &mut TestAppContext,
) {
    let comments = vec![comment('1', Author::Reviewer), comment('2', Author::Agent)];
    let (mut stage, mut cx) = stage_with_thread_in_view(20, comments, 1, cx);
    let held = reading(&stage, &mut cx);
    assert!(
        matches!(held.first, PaneRow::Comment { .. }),
        "a comment is first in view"
    );
    let (diff, offset) = diff_in_view(&stage, &mut cx);
    let offset = offset as isize;

    // The row first in view, with the thread's other two rows, goes: the first row of the
    // diff in view takes its place as what the view is kept by.
    tell_threads(&mut stage, Vec::new(), &mut cx);
    assert_eq!(below_the_top(&diff, &stage, &mut cx), offset);
    let after = reading(&stage, &mut cx);
    assert_eq!(after.start, held.start - 3, "the thread's three rows left");
    assert_eq!(after.position, held.position);
}

#[gpui_kit::test]
fn with_nothing_but_a_thread_in_view_the_thread_is_what_the_view_is_kept_by(
    cx: &mut TestAppContext,
) {
    // More comments than rows in view, and the view among them.
    let long = comments(40);
    let (mut stage, mut cx) = stage_with_thread_in_view(10, long.clone(), 6, cx);
    let held = reading(&stage, &mut cx);
    let in_view = stage.pane.read_with(&cx, |pane, _| pane.rows_in_view());
    let only_comments = stage.pane.read_with(&cx, |pane, _| {
        pane.rows()[in_view.clone()]
            .iter()
            .all(|row| matches!(row, PaneRow::Comment { .. }))
    });
    assert!(only_comments, "only comments are in view: {in_view:?}");

    // The thread moves a line down: a row of the diff comes above it, and the comments
    // stay where they were on screen.
    tell_moved(&mut stage, thread_on(11, long), 11, &mut cx);
    let after = reading(&stage, &mut cx);
    assert_eq!(after.view, View::Changes);
    assert_eq!(after.first, held.first);
    assert_eq!(after.start, held.start + 1, "a row of the diff came above");
    assert_eq!(after.position, held.position);
}

#[gpui_kit::test]
fn when_the_thread_first_in_view_goes_the_next_one_in_view_is_what_the_view_is_kept_by(
    cx: &mut TestAppContext,
) {
    // Two long threads on one line, one after the other, and the view among the last
    // comments of the first and the first of the second: nothing but threads and comments
    // is in view.
    let second = || thread_with('8', 10, comments_of('8', 40));
    let (mut stage, mut cx) = stage_with_threads(
        vec![thread_with('7', 10, comments_of('7', 40)), second()],
        cx,
    );
    let at = |digit: char, stage: &Stage, cx: &mut VisualTestContext| {
        let wanted = ThreadId::new(digit.to_string().repeat(32));
        rows_of(stage, cx)
            .iter()
            .position(|row| matches!(row, PaneRow::Thread { thread } if *thread == wanted))
            .expect("the thread has a row")
    };
    let first_row = at('7', &stage, &mut cx);
    scroll_until_first(&stage, first_row + 30, &mut cx);
    let held = reading(&stage, &mut cx);
    assert!(
        matches!(held.first, PaneRow::Comment { .. }),
        "a comment of the first thread is first in view"
    );
    let second_row = rows_of(&stage, &mut cx)[at('8', &stage, &mut cx)].clone();
    let offset = below_the_top(&second_row, &stage, &mut cx);
    assert!(
        offset > 0,
        "the second thread starts in view, {offset} rows down"
    );
    let in_view = stage.pane.read_with(&cx, |pane, _| pane.rows_in_view());
    assert!(
        rows_of(&stage, &mut cx)[in_view.clone()]
            .iter()
            .all(|row| matches!(row, PaneRow::Thread { .. } | PaneRow::Comment { .. })),
        "only threads and comments are in view: {in_view:?}"
    );

    // The first thread goes, with every row of it in view: the second thread's row is what
    // the view is kept by, and the second thread stays where it was on screen.
    tell_threads(&mut stage, vec![second()], &mut cx);
    assert_eq!(below_the_top(&second_row, &stage, &mut cx), offset);
    assert_eq!(reading(&stage, &mut cx).view, View::Changes);
}

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
use std::time::{Duration, Instant};

use gpui_kit::{Modifiers, ScrollDelta, ScrollEvent, TestAppContext, VisualTestContext, point, px};
use yaar_link::{
    Anchor, Author, Comment, CommentId, Fact, FileStatus, Lines, ReviewId, SessionState,
    Side as LinkSide, Step, Thread, ThreadId, WorkspaceReviews,
};
use yaar_review_pane::{PaneRow, Position, View};

use common::{
    Stage, agents_of, bounds_of_row, drawn, file, id, open_staged, review_with, reviews_of,
    running_turn, session, session_id, since_main, step_fact, texts, turn_id, wait_for_files,
    wait_until, working_on, workspace,
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

/// The thread of the review, on line `line` (counted from 1) of `long.txt`, new side.
fn thread_on(line: u32, comments: Vec<Comment>) -> Thread {
    Thread {
        id: ThreadId::new("7".repeat(32)),
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
    let mut review = review_with(review_id(), Some(session_id('a')), None);
    review.threads.push(thread);
    reviews_of(vec![review])
}

/// What the reviewer reads: the first row in view, where it is, and the line they are on.
#[derive(Debug, PartialEq)]
struct Reading {
    first: PaneRow,
    start: usize,
    position: Option<Position>,
}

fn reading(stage: &Stage, cx: &mut VisualTestContext) -> Reading {
    cx.run_until_parked();
    stage.pane.read_with(cx, |pane, _| {
        let start = pane.rows_in_view().start;
        Reading {
            first: pane.rows()[start].clone(),
            start,
            position: pane.position().cloned(),
        }
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

/// Keeps the app running until the element with this selector is drawn.
fn wait_drawn(selector: &str, cx: &mut VisualTestContext) {
    let deadline = Instant::now() + common::DEADLINE;
    while drawn(selector, cx).is_none() {
        cx.run_until_parked();
        assert!(
            Instant::now() < deadline,
            "waited in vain for {selector} to be drawn"
        );
        std::thread::park_timeout(Duration::from_millis(1));
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
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.view()),
        View::Changes
    );
    assert_eq!(reading(&stage, &mut cx), held, "after the steps");

    // A turn end that leaves the selection as it is: the offer is drawn, and nothing else.
    stage.keeper.tell(Fact::TurnEnded {
        workspace: id('1'),
        session: session_id('a'),
        turn: turn.clone(),
        state: Some("6".repeat(40)),
        since: None,
    });
    wait_drawn("review-offer", &mut cx);
    assert_eq!(reading(&stage, &mut cx), held, "after the turn end");

    // A reply: one more comment in the thread, above the rows in view. The row that was
    // first in view is first again, one row further down the list.
    stage.keeper.set_reviews(review_of_a(thread_on(
        3,
        vec![comment('1', Author::Reviewer), comment('2', Author::Agent)],
    )));
    stage
        .keeper
        .tell(Fact::ReviewsChanged { workspace: id('1') });
    wait_until(&mut cx, "the reply", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.reviews()
                .is_some_and(|reviews| reviews.reviews[0].threads[0].comments.len() == 2)
        })
    });
    let after_reply = reading(&stage, &mut cx);
    assert_eq!(after_reply.first, held.first);
    assert_eq!(after_reply.start, held.start + 1, "a row came above it");
    assert_eq!(after_reply.position, held.position);

    // A thread that moved from line 3 to line 4, still above the rows in view.
    let before = thread_row(&stage, &mut cx);
    stage.keeper.set_reviews(review_of_a(thread_on(
        4,
        vec![comment('1', Author::Reviewer), comment('2', Author::Agent)],
    )));
    stage
        .keeper
        .tell(Fact::ReviewsChanged { workspace: id('1') });
    wait_until(&mut cx, "the thread on its new line", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.reviews()
                .is_some_and(|reviews| reviews.reviews[0].threads[0].anchor.lines.first == 4)
        })
    });
    assert_eq!(
        thread_row(&stage, &mut cx),
        before + 1,
        "its rows moved down"
    );
    assert_eq!(reading(&stage, &mut cx), after_reply);
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

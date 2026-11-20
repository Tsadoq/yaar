//! Component test: attaching a review, the states of its threads, and what only the
//! reviewer does to threads, in the Review pane on GPUI's test platform, with no window on
//! any screen. Keys, clicks and the wheel go in; the pane is read back through the model it
//! shows. The test plays the Keeper on the other end of a socket pair as the Keeper of
//! slice 6 plays it: what the Keeper works out (a thread's state, what is attached) is set
//! as the data the Link carries, and a new answer to `GetReviews` comes after
//! `Fact::ReviewsChanged`.
//!
//! A negative clause ("sends nothing", "not drawn") is read after a barrier: something the
//! test causes after it, through the same path, whose arrival shows that everything before
//! it arrived.

mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::{Modifiers, TestAppContext, VisualTestContext, px, size};
use yaar_diff::Side;
use yaar_frame::Wanted;
use yaar_link::{
    AgentCli, Anchor, Attachment, Author, Comment, CommentId, Fact, FileStatus, Lines, Refusal,
    Reply, Request, ReviewId, Session, SessionState, Side as LinkSide, Thread, ThreadId,
    ThreadState, WorkspaceReviews,
};
use yaar_review_pane::PaneRow;

use common::{
    Stage, WINDOW, agents_of, bounds_of_row, drawn, file, id, no_agent_review, open_staged,
    review_with, reviews_of, session, session_id, since_main, texts, wait_for_files, wait_until,
    workspace,
};

fn review_id() -> ReviewId {
    ReviewId::new("1".repeat(32))
}

fn thread_id(digit: char) -> ThreadId {
    ThreadId::new(digit.to_string().repeat(32))
}

fn claude() -> AgentCli {
    AgentCli {
        agent: "claude".to_owned(),
        title: "Claude Code".to_owned(),
    }
}

fn pi() -> AgentCli {
    AgentCli {
        agent: "pi".to_owned(),
        title: "pi".to_owned(),
    }
}

/// A thread of the reviewer on the new side of `path` at `line`, in the state `state`: a
/// draft has a comment that was not sent, the others one of round 1.
fn thread(digit: char, path: &str, line: u32, state: ThreadState) -> Thread {
    let draft = state == ThreadState::Draft;
    let mut comments = vec![Comment {
        id: CommentId::new(digit.to_string().repeat(32)),
        author: Author::Reviewer,
        text: format!("comment {digit}"),
        round: (!draft).then_some(1),
    }];
    if matches!(
        state,
        ThreadState::Addressed | ThreadState::WaitsForReviewer
    ) {
        comments.push(Comment {
            id: CommentId::new(format!("a{}", digit.to_string().repeat(31))),
            author: Author::Agent,
            text: "an answer".to_owned(),
            round: Some(1),
        });
    }
    Thread {
        id: thread_id(digit),
        anchor: Anchor {
            tree: None,
            path: PathBuf::from(path),
            side: LinkSide::New,
            lines: Lines {
                first: line,
                last: line,
            },
            text: format!("line {line}"),
        },
        resolved: state == ThreadState::Resolved,
        addressed: state == ThreadState::Addressed,
        comments,
        state,
        now: None,
    }
}

/// The session `a` of the tests, in this state, with the review of the tests.
fn session_a(state: SessionState) -> Session {
    session('a', "Claude Code 1", state, Some(review_id()))
}

/// The review of session `a` with these threads, attached as given.
fn review_of_a(threads: Vec<Thread>, attached: Option<Attachment>) -> WorkspaceReviews {
    let mut review = review_with(review_id(), Some(session_id('a')), None);
    review.threads = threads;
    review.attached = attached;
    reviews_of(vec![review])
}

const OLD: &str = "a\nb\nc\nd\ne\n";
const NEW: &str = "a\nB\nC\nD\nE\n";

/// A window with the pane in it, that shows the changes of workspace `1` against its base
/// (`story.txt` from [`OLD`] to [`NEW`]), with the review of session `a`, which is in
/// this state.
fn open_on_story(
    reviews: WorkspaceReviews,
    state: SessionState,
    cx: &mut TestAppContext,
) -> (Stage, VisualTestContext) {
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature")],
        |keeper| {
            keeper.set_agents(vec![agents_of(vec![session_a(state)])]);
            keeper.set_reviews(reviews);
        },
        cx,
    );
    let changes = since_main(Vec::new(), vec![file("story.txt", FileStatus::Modified)]);
    stage.keeper.show(&changes, |_| texts(OLD, NEW), &mut cx);
    wait_for_files(&stage.pane, &mut cx);
    wait_until(&mut cx, "the review of session a", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.review()
                .is_some_and(|review| review.agent == Some(session_id('a')))
        })
    });
    // Room for the rows of every thread, so that all are laid out.
    cx.simulate_resize(size(WINDOW.width, px(800.)));
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
    cx.run_until_parked();
}

/// The Keeper tells that the reviews changed, to these, and the test waits until the pane
/// holds them.
fn tell_reviews(stage: &mut Stage, reviews: WorkspaceReviews, cx: &mut VisualTestContext) {
    let held = reviews.reviews.clone();
    stage.keeper.set_reviews(reviews);
    stage
        .keeper
        .tell(Fact::ReviewsChanged { workspace: id('1') });
    wait_until(cx, "the reviews as told", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.reviews()
                .is_some_and(|reviews| reviews.reviews == held)
        })
    });
}

/// Puts the reviewer on the line the row before the row of `thread` shows, by a click that
/// also gives the pane the focus: the line the thread sits on.
fn go_to_the_line_of(stage: &Stage, thread: &ThreadId, cx: &mut VisualTestContext) {
    let row = stage
        .pane
        .read_with(cx, |pane, _| {
            pane.rows()
                .iter()
                .position(|row| matches!(row, PaneRow::Thread { thread: held } if held == thread))
        })
        .unwrap_or_else(|| panic!("no row for the thread {thread}"));
    let line = bounds_of_row(row - 1, cx).center();
    cx.simulate_click(line, Modifiers::none());
    cx.update(|window, _| assert!(stage.focus.is_focused(window)));
}

fn notice(stage: &Stage, cx: &mut VisualTestContext) -> Option<String> {
    stage.pane.read_with(cx, |pane, _| pane.notice())
}

/// Answers the next request, which has to be this one, with a yes.
fn expect_done(stage: &mut Stage, wanted: Request, cx: &mut VisualTestContext) {
    let asked = stage.keeper.expect(wanted, cx);
    stage.keeper.reply(asked, Ok(Reply::Done));
}

/// The terminals the pane wants opened, as the shared list of terminals tells them.
fn wanted_terminals(stage: &Stage, cx: &mut VisualTestContext) -> Rc<RefCell<Vec<Wanted>>> {
    let heard = Rc::new(RefCell::new(Vec::new()));
    let sink = heard.clone();
    let terminals = stage.models.terminals.clone();
    cx.update(|_, cx| {
        cx.subscribe(&terminals, move |_, wanted: &Wanted, _| {
            sink.borrow_mut().push(wanted.clone());
        })
        .detach();
    });
    heard
}

#[gpui_kit::test]
fn the_pane_lists_all_threads_each_with_its_state_and_only_the_reviewer_resolves_reopens_and_resolves_every_addressed_one(
    cx: &mut TestAppContext,
) {
    // T1 draft, T2 sent, T3 addressed, T4 waiting for the reviewer (on a file the diff
    // does not show), T5 resolved: four on lines of the diff and one elsewhere.
    let threads = vec![
        thread('1', "story.txt", 2, ThreadState::Draft),
        thread('2', "story.txt", 3, ThreadState::Sent),
        thread('3', "story.txt", 4, ThreadState::Addressed),
        thread('4', "other.txt", 1, ThreadState::WaitsForReviewer),
        thread('5', "story.txt", 5, ThreadState::Resolved),
    ];
    let (mut stage, mut cx) = open_on_story(review_of_a(threads, None), SessionState::Idle, cx);

    // The pane lists all threads: each has its row, on a line or under the heading of the
    // threads on no line.
    let rows = stage.pane.read_with(&cx, |pane, _| pane.rows().to_vec());
    for digit in ['1', '2', '3', '4', '5'] {
        assert!(
            rows.contains(&PaneRow::Thread {
                thread: thread_id(digit)
            }),
            "thread {digit} has no row"
        );
    }
    let elsewhere = rows
        .iter()
        .position(|row| *row == PaneRow::ThreadsElsewhere)
        .expect("the thread on a file not shown is listed under its heading");
    assert!(rows[elsewhere..].contains(&PaneRow::Thread {
        thread: thread_id('4')
    }));

    // Each with its state, as the model says it and as the row draws it.
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.thread_states()),
        vec![
            (thread_id('1'), ThreadState::Draft),
            (thread_id('2'), ThreadState::Sent),
            (thread_id('3'), ThreadState::Addressed),
            (thread_id('4'), ThreadState::WaitsForReviewer),
            (thread_id('5'), ThreadState::Resolved),
        ]
    );
    for (digit, mark) in [
        ('1', "draft"),
        ('2', "sent"),
        ('3', "addressed"),
        ('4', "waiting"),
        ('5', "resolved"),
    ] {
        let selector = format!("review-thread-state-{}:{mark}", thread_id(digit));
        assert!(is_drawn(&selector, &mut cx), "{selector} is not drawn");
    }
    // A row carries its own state and no other.
    assert!(!is_drawn(
        &format!("review-thread-state-{}:sent", thread_id('3')),
        &mut cx
    ));

    // The Keeper tells that the agent replied to T2 with the addressed mark: the mark of
    // its row changes.
    let mut answered = thread('2', "story.txt", 3, ThreadState::Addressed);
    answered.comments.truncate(1);
    let threads = vec![
        thread('1', "story.txt", 2, ThreadState::Draft),
        answered,
        thread('3', "story.txt", 4, ThreadState::Addressed),
        thread('4', "other.txt", 1, ThreadState::WaitsForReviewer),
        thread('5', "story.txt", 5, ThreadState::Resolved),
    ];
    tell_reviews(&mut stage, review_of_a(threads, None), &mut cx);
    assert!(is_drawn(
        &format!("review-thread-state-{}:addressed", thread_id('2')),
        &mut cx
    ));
    assert!(!is_drawn(
        &format!("review-thread-state-{}:sent", thread_id('2')),
        &mut cx
    ));

    // The reviewer on T2's line: Resolve Thread asks the Keeper to resolve it.
    go_to_the_line_of(&stage, &thread_id('2'), &mut cx);
    cx.simulate_keystrokes("x");
    expect_done(
        &mut stage,
        Request::ResolveThread {
            review: review_id(),
            thread: thread_id('2'),
        },
        &mut cx,
    );
    // On T5's: Reopen Thread asks the Keeper to reopen it.
    go_to_the_line_of(&stage, &thread_id('5'), &mut cx);
    cx.simulate_keystrokes("u");
    expect_done(
        &mut stage,
        Request::ReopenThread {
            review: review_id(),
            thread: thread_id('5'),
        },
        &mut cx,
    );
    // Wherever the reviewer is: Resolve Addressed.
    cx.simulate_keystrokes("shift-x");
    expect_done(
        &mut stage,
        Request::ResolveAddressed {
            review: review_id(),
        },
        &mut cx,
    );
    // The pane changed no state itself: each shows as the Keeper last said.
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.thread_states())[4],
        (thread_id('5'), ThreadState::Resolved)
    );
    assert_eq!(notice(&stage, &mut cx), None);
}

#[gpui_kit::test]
fn resolving_reopening_and_attaching_a_thread_need_the_reviewer_on_a_thread_that_allows_it_and_say_so_in_the_notice_otherwise(
    cx: &mut TestAppContext,
) {
    let threads = vec![
        thread('2', "story.txt", 3, ThreadState::Sent),
        thread('5', "story.txt", 5, ThreadState::Resolved),
    ];
    let (mut stage, mut cx) = open_on_story(review_of_a(threads, None), SessionState::Idle, cx);
    // Rows of the diff with no thread on them: line 1 of story.txt, which is no change,
    // is the first row of the diff after the heading of the file.
    let rows = stage.pane.read_with(&cx, |pane, _| pane.rows().to_vec());
    let bare = rows
        .iter()
        .position(|row| matches!(row, PaneRow::Diff { row, .. } if row.line(Side::New) == Some(0)))
        .expect("the first line of the file has a row");
    let bare_line = bounds_of_row(bare, &mut cx).center();
    cx.simulate_click(bare_line, Modifiers::none());
    cx.update(|window, _| assert!(stage.focus.is_focused(window)));

    for key in ["x", "u", "t"] {
        cx.simulate_keystrokes(key);
        assert!(
            notice(&stage, &mut cx).is_some_and(|words| words.contains("thread")),
            "{key} on a line with no thread said nothing about it"
        );
    }

    // On a resolved thread, Resolve Thread and Attach Thread say so; on an unresolved one,
    // Reopen Thread says so.
    go_to_the_line_of(&stage, &thread_id('5'), &mut cx);
    for key in ["x", "t"] {
        cx.simulate_keystrokes(key);
        assert!(
            notice(&stage, &mut cx).is_some_and(|words| words.contains("resolved")),
            "{key} on a resolved thread said nothing about it"
        );
    }
    go_to_the_line_of(&stage, &thread_id('2'), &mut cx);
    cx.simulate_keystrokes("u");
    assert!(notice(&stage, &mut cx).is_some_and(|words| words.contains("not resolved")));

    // Attach Thread on a thread that holds a comment to send: the thread alone.
    cx.simulate_keystrokes("t");
    let asked = stage.keeper.expect(
        Request::AttachReview {
            review: review_id(),
            thread: Some(thread_id('2')),
        },
        &mut cx,
    );
    // Barrier: none of the keys above sent anything, or this would not be the next
    // request the Keeper heard.
    stage
        .keeper
        .reply(asked, Ok(Reply::AttachedReview { start: Vec::new() }));
}

#[gpui_kit::test]
fn what_is_attached_of_the_review_shown_is_drawn_from_the_reviews_the_keeper_holds_and_a_detach_asks_the_keeper_to_take_it_away(
    cx: &mut TestAppContext,
) {
    let threads = vec![thread('1', "story.txt", 2, ThreadState::Draft)];
    // A pane that opens on a review that was attached before: as after a restart.
    let (mut stage, mut cx) = open_on_story(
        review_of_a(threads.clone(), Some(Attachment::Review)),
        SessionState::Idle,
        cx,
    );
    assert!(is_drawn("review-attached:review", &mut cx));
    assert!(!is_drawn("review-attached:thread", &mut cx));
    // Nothing was attached by a key, so no offer is made.
    assert!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.offer_to_start())
            .is_empty()
    );
    assert!(!is_drawn("review-offer-start", &mut cx));

    // One thread alone.
    tell_reviews(
        &mut stage,
        review_of_a(threads.clone(), Some(Attachment::Thread(thread_id('1')))),
        &mut cx,
    );
    assert!(is_drawn("review-attached:thread", &mut cx));
    assert!(!is_drawn("review-attached:review", &mut cx));

    // Detach Review asks the Keeper to take it away; the line goes when the Keeper says
    // the reviews changed.
    cx.update(|window, cx| stage.focus.focus(window, cx));
    cx.simulate_keystrokes("shift-d");
    expect_done(
        &mut stage,
        Request::DetachReview {
            review: review_id(),
        },
        &mut cx,
    );
    tell_reviews(&mut stage, review_of_a(threads, None), &mut cx);
    assert!(!is_drawn("review-attached:thread", &mut cx));
    assert!(!is_drawn("review-attached:review", &mut cx));
}

#[gpui_kit::test]
fn an_attach_the_keeper_refuses_says_why_in_the_notice_and_draws_nothing(cx: &mut TestAppContext) {
    let threads = vec![thread('2', "story.txt", 3, ThreadState::Sent)];
    let (mut stage, mut cx) = open_on_story(review_of_a(threads, None), SessionState::Idle, cx);
    cx.update(|window, cx| stage.focus.focus(window, cx));
    cx.simulate_keystrokes("a");
    let asked = stage.keeper.expect(
        Request::AttachReview {
            review: review_id(),
            thread: None,
        },
        &mut cx,
    );
    stage.keeper.reply(
        asked,
        Err(Refusal::NothingToSend {
            review: review_id(),
        }),
    );
    wait_until(&mut cx, "the notice", |cx| {
        stage.pane.read_with(cx, |pane, _| pane.notice().is_some())
    });
    let words = notice(&stage, &mut cx).expect("the notice");
    assert!(
        words.contains("unsent comment") && !words.contains("copy"),
        "{words}"
    );
    assert!(!is_drawn("review-attached:review", &mut cx));
    assert!(!is_drawn("review-offer-start", &mut cx));
}

#[gpui_kit::test]
fn attaching_a_review_whose_agent_is_not_running_shows_the_offer_to_start_it_and_attaching_that_of_no_agent_shows_each_supported_cli(
    cx: &mut TestAppContext,
) {
    let threads = vec![thread('1', "story.txt", 2, ThreadState::Draft)];
    let (mut stage, mut cx) = open_on_story(
        review_of_a(threads.clone(), None),
        SessionState::NotRunning,
        cx,
    );
    let wanted = wanted_terminals(&stage, &mut cx);
    cx.update(|window, cx| stage.focus.focus(window, cx));

    // The review of session a, which does not run: `a` attaches it, and the answer names
    // the CLI to start. The Keeper tells the review attached after it.
    cx.simulate_keystrokes("a");
    let asked = stage.keeper.expect(
        Request::AttachReview {
            review: review_id(),
            thread: None,
        },
        &mut cx,
    );
    stage.keeper.reply(
        asked,
        Ok(Reply::AttachedReview {
            start: vec![claude()],
        }),
    );
    tell_reviews(
        &mut stage,
        review_of_a(threads.clone(), Some(Attachment::Review)),
        &mut cx,
    );
    wait_until(&mut cx, "the offer", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.offer_to_start() == [claude()])
    });
    assert!(is_drawn("review-offer-start", &mut cx));
    assert!(is_drawn("review-offer-start-claude", &mut cx));
    assert!(!is_drawn("review-offer-start-pi", &mut cx));

    // A click on the control asks the terminal pane, through the shared list of
    // terminals, to open the launcher of that CLI in the workspace for the review.
    click("review-offer-start-claude", &mut cx);
    let want = Wanted::AgentFor {
        workspace: id('1'),
        agent: "claude".to_owned(),
        review: review_id(),
    };
    assert_eq!(*wanted.borrow(), std::slice::from_ref(&want));
    // The offer is taken: it goes.
    assert!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.offer_to_start())
            .is_empty()
    );
    assert!(!is_drawn("review-offer-start", &mut cx));

    // Attached again, Start Agent takes the first CLI offered: the same wish.
    cx.simulate_keystrokes("a");
    let asked = stage.keeper.expect(
        Request::AttachReview {
            review: review_id(),
            thread: None,
        },
        &mut cx,
    );
    stage.keeper.reply(
        asked,
        Ok(Reply::AttachedReview {
            start: vec![claude()],
        }),
    );
    wait_until(&mut cx, "the offer again", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.offer_to_start() == [claude()])
    });
    cx.simulate_keystrokes("shift-n");
    assert_eq!(*wanted.borrow(), [want.clone(), want]);
    assert!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.offer_to_start())
            .is_empty()
    );
}

#[gpui_kit::test]
fn attaching_the_review_of_no_agent_shows_the_offer_of_each_cli_and_start_agent_takes_the_first(
    cx: &mut TestAppContext,
) {
    let mut review = no_agent_review();
    review.threads = vec![thread('1', "story.txt", 2, ThreadState::Draft)];
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature")],
        |keeper| keeper.set_reviews(reviews_of(vec![review.clone()])),
        cx,
    );
    let changes = since_main(Vec::new(), vec![file("story.txt", FileStatus::Modified)]);
    stage.keeper.show(&changes, |_| texts(OLD, NEW), &mut cx);
    wait_for_files(&stage.pane, &mut cx);
    wait_until(&mut cx, "the review of no agent", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.review().is_some_and(|r| !r.threads.is_empty())
        })
    });
    let wanted = wanted_terminals(&stage, &mut cx);
    cx.update(|window, cx| stage.focus.focus(window, cx));

    // A list of two CLIs is a shape the Keeper of this slice, with one Adapter, never
    // sends: it is allowed here to show the drawing of a list.
    cx.simulate_keystrokes("a");
    let asked = stage.keeper.expect(
        Request::AttachReview {
            review: review.id.clone(),
            thread: None,
        },
        &mut cx,
    );
    stage.keeper.reply(
        asked,
        Ok(Reply::AttachedReview {
            start: vec![claude(), pi()],
        }),
    );
    review.attached = Some(Attachment::Review);
    tell_reviews(&mut stage, reviews_of(vec![review.clone()]), &mut cx);
    wait_until(&mut cx, "the offer", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.offer_to_start() == [claude(), pi()])
    });
    assert!(is_drawn("review-offer-start", &mut cx));
    assert!(is_drawn("review-offer-start-claude", &mut cx));
    assert!(is_drawn("review-offer-start-pi", &mut cx));

    // The second control starts the second CLI.
    let started = |agent: &str| Wanted::AgentFor {
        workspace: id('1'),
        agent: agent.to_owned(),
        review: review.id.clone(),
    };
    click("review-offer-start-pi", &mut cx);
    assert_eq!(*wanted.borrow(), [started("pi")]);

    // Offered again, Start Agent takes the first.
    cx.simulate_keystrokes("a");
    let asked = stage.keeper.expect(
        Request::AttachReview {
            review: review.id.clone(),
            thread: None,
        },
        &mut cx,
    );
    stage.keeper.reply(
        asked,
        Ok(Reply::AttachedReview {
            start: vec![claude(), pi()],
        }),
    );
    wait_until(&mut cx, "the offer again", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.offer_to_start().len() == 2)
    });
    cx.simulate_keystrokes("shift-n");
    assert_eq!(*wanted.borrow(), [started("pi"), started("claude")]);
}

#[gpui_kit::test]
fn attaching_the_review_of_an_agent_that_runs_offers_nothing_and_an_offer_goes_when_nothing_is_attached_any_more(
    cx: &mut TestAppContext,
) {
    let threads = vec![thread('1', "story.txt", 2, ThreadState::Draft)];
    let (mut stage, mut cx) =
        open_on_story(review_of_a(threads.clone(), None), SessionState::Idle, cx);
    cx.update(|window, cx| stage.focus.focus(window, cx));
    cx.simulate_keystrokes("a");
    let asked = stage.keeper.expect(
        Request::AttachReview {
            review: review_id(),
            thread: None,
        },
        &mut cx,
    );
    stage
        .keeper
        .reply(asked, Ok(Reply::AttachedReview { start: Vec::new() }));
    // The barrier: the attach shows in the reviews the Keeper tells after the answer, so
    // the answer was taken before the pane holds them.
    tell_reviews(
        &mut stage,
        review_of_a(threads.clone(), Some(Attachment::Review)),
        &mut cx,
    );
    assert!(is_drawn("review-attached:review", &mut cx));
    assert!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.offer_to_start())
            .is_empty()
    );
    assert!(!is_drawn("review-offer-start", &mut cx));

    // With no offer, Start Agent says so and asks for no terminal: the pane's own request
    // that follows is the barrier.
    let wanted = wanted_terminals(&stage, &mut cx);
    cx.simulate_keystrokes("shift-n");
    assert!(notice(&stage, &mut cx).is_some_and(|words| words.contains("offer")));
    cx.simulate_keystrokes("shift-x");
    expect_done(
        &mut stage,
        Request::ResolveAddressed {
            review: review_id(),
        },
        &mut cx,
    );
    assert!(wanted.borrow().is_empty());

    // The agent stops. An offer, then what is attached goes: the offer goes with it.
    stage
        .keeper
        .tell_agents(agents_of(vec![session_a(SessionState::NotRunning)]));
    wait_until(&mut cx, "the session stopped", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.sessions()
                .iter()
                .any(|session| session.state == SessionState::NotRunning)
        })
    });
    cx.simulate_keystrokes("a");
    let asked = stage.keeper.expect(
        Request::AttachReview {
            review: review_id(),
            thread: None,
        },
        &mut cx,
    );
    stage.keeper.reply(
        asked,
        Ok(Reply::AttachedReview {
            start: vec![claude()],
        }),
    );
    wait_until(&mut cx, "the offer", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| !pane.offer_to_start().is_empty())
    });
    tell_reviews(&mut stage, review_of_a(threads.clone(), None), &mut cx);
    assert!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.offer_to_start())
            .is_empty()
    );
    assert!(!is_drawn("review-offer-start", &mut cx));

    // Attached again from elsewhere, with nothing asked here: the offer that went does not
    // come back.
    let mut again = review_of_a(threads, Some(Attachment::Review));
    again.reviews[0]
        .threads
        .push(thread('2', "story.txt", 3, ThreadState::Draft));
    tell_reviews(&mut stage, again, &mut cx);
    assert!(is_drawn("review-attached:review", &mut cx));
    assert!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.offer_to_start())
            .is_empty()
    );
    assert!(!is_drawn("review-offer-start", &mut cx));
}

#[gpui_kit::test]
fn an_offer_made_while_the_agent_did_not_run_is_not_shown_once_it_runs_and_an_answer_naming_no_cli_replaces_it(
    cx: &mut TestAppContext,
) {
    let threads = vec![thread('1', "story.txt", 2, ThreadState::Draft)];
    // Another thread, so that each reviews told below differs from the one before and the
    // test can wait for it.
    let more = vec![
        threads[0].clone(),
        thread('2', "story.txt", 3, ThreadState::Draft),
    ];
    let (mut stage, mut cx) = open_on_story(
        review_of_a(threads.clone(), None),
        SessionState::NotRunning,
        cx,
    );
    cx.update(|window, cx| stage.focus.focus(window, cx));

    // The agent does not run: the offer is made.
    let attach = |answer: Vec<AgentCli>, stage: &mut Stage, cx: &mut VisualTestContext| {
        cx.simulate_keystrokes("a");
        let asked = stage.keeper.expect(
            Request::AttachReview {
                review: review_id(),
                thread: None,
            },
            cx,
        );
        stage
            .keeper
            .reply(asked, Ok(Reply::AttachedReview { start: answer }));
    };
    attach(vec![claude()], &mut stage, &mut cx);
    tell_reviews(
        &mut stage,
        review_of_a(threads, Some(Attachment::Review)),
        &mut cx,
    );
    wait_until(&mut cx, "the offer", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.offer_to_start() == [claude()])
    });
    assert!(is_drawn("review-offer-start", &mut cx));

    // The agent is started some other way: its CLI is not offered to start any more.
    stage
        .keeper
        .tell_agents(agents_of(vec![session_a(SessionState::Idle)]));
    wait_until(&mut cx, "the session running", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.sessions()
                .iter()
                .any(|session| session.state == SessionState::Idle)
        })
    });
    assert!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.offer_to_start())
            .is_empty()
    );
    assert!(!is_drawn("review-offer-start", &mut cx));

    // Attached again, the answer names no CLI: it replaces the offer that was made, which
    // does not come back when the agent stops.
    attach(Vec::new(), &mut stage, &mut cx);
    tell_reviews(
        &mut stage,
        review_of_a(more, Some(Attachment::Review)),
        &mut cx,
    );
    stage
        .keeper
        .tell_agents(agents_of(vec![session_a(SessionState::NotRunning)]));
    wait_until(&mut cx, "the session stopped", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.sessions()
                .iter()
                .any(|session| session.state == SessionState::NotRunning)
        })
    });
    assert!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.offer_to_start())
            .is_empty()
    );
    assert!(!is_drawn("review-offer-start", &mut cx));
}

/// Puts workspace `digit` on screen, with the change of `story.txt` the Keeper holds for
/// it, and waits for its files.
fn go_to_workspace(digit: char, stage: &mut Stage, cx: &mut VisualTestContext) {
    stage
        .models
        .workspaces
        .update(cx, |list, cx| list.select(&id(digit), cx));
    let changes = yaar_link::Changes {
        workspace: id(digit),
        ..since_main(Vec::new(), vec![file("story.txt", FileStatus::Modified)])
    };
    stage.keeper.show(&changes, |_| texts(OLD, NEW), cx);
    wait_for_files(&stage.pane, cx);
}

/// The reviews of the next-review tests: that of no agent, and those of sessions `a` and
/// `b`.
fn three_reviews() -> (ReviewId, ReviewId, WorkspaceReviews) {
    let b = ReviewId::new("2".repeat(32));
    let none = no_agent_review();
    let reviews = reviews_of(vec![
        review_with(review_id(), Some(session_id('a')), None),
        review_with(b.clone(), Some(session_id('b')), None),
        none.clone(),
    ]);
    (none.id, b, reviews)
}

#[gpui_kit::test]
fn next_review_and_the_session_bar_reach_the_review_of_no_agent_and_come_back_from_it(
    cx: &mut TestAppContext,
) {
    let (none, b, reviews) = three_reviews();
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature")],
        |keeper| {
            keeper.set_agents(vec![agents_of(vec![
                session('a', "Claude Code 1", SessionState::Idle, Some(review_id())),
                session('b', "Claude Code 2", SessionState::Idle, Some(b.clone())),
            ])]);
            keeper.set_reviews(reviews);
        },
        cx,
    );
    let changes = since_main(Vec::new(), vec![file("story.txt", FileStatus::Modified)]);
    stage.keeper.show(&changes, |_| texts(OLD, NEW), &mut cx);
    wait_for_files(&stage.pane, &mut cx);
    let shown = |stage: &Stage, cx: &mut VisualTestContext| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.review().map(|review| review.id.clone()))
    };
    wait_until(&mut cx, "both sessions and the reviews", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.sessions().len() == 2 && pane.review().is_some()
        })
    });
    // The review of no agent is shown first; the bar names it after the sessions.
    assert_eq!(shown(&stage, &mut cx), Some(none.clone()));
    assert!(is_drawn("review-session-name-0:Claude Code 1", &mut cx));
    assert!(is_drawn("review-session-name-1:Claude Code 2", &mut cx));
    assert!(is_drawn("review-session-name-2:No agent", &mut cx));
    assert!(!is_drawn("review-session-3", &mut cx));

    // Next Review goes through the sessions and on to the review of no agent, and from it
    // to the first session again.
    cx.update(|window, cx| stage.focus.focus(window, cx));
    for wanted in [review_id(), b.clone(), none.clone(), review_id(), b.clone()] {
        cx.simulate_keystrokes("shift-a");
        assert_eq!(shown(&stage, &mut cx), Some(wanted));
    }

    // A click on the name of the review of no agent shows it.
    click("review-session-2", &mut cx);
    assert_eq!(shown(&stage, &mut cx), Some(none));
    click("review-session-0", &mut cx);
    assert_eq!(shown(&stage, &mut cx), Some(review_id()));
}

#[gpui_kit::test]
fn an_offer_is_made_for_one_review_and_goes_when_another_review_or_another_workspace_is_shown(
    cx: &mut TestAppContext,
) {
    let (none, b, mut reviews) = three_reviews();
    // Both the review of no agent and that of session `a`, the one after it, are attached.
    for review in &mut reviews.reviews {
        if review.id == none || review.id == review_id() {
            review.attached = Some(Attachment::Review);
        }
    }
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature"), workspace('2', "other")],
        |keeper| {
            keeper.set_agents(vec![agents_of(vec![
                session('a', "Claude Code 1", SessionState::Idle, Some(review_id())),
                session('b', "Claude Code 2", SessionState::Idle, Some(b.clone())),
            ])]);
            keeper.set_reviews(reviews);
        },
        cx,
    );
    let changes = since_main(Vec::new(), vec![file("story.txt", FileStatus::Modified)]);
    stage.keeper.show(&changes, |_| texts(OLD, NEW), &mut cx);
    wait_for_files(&stage.pane, &mut cx);
    wait_until(&mut cx, "the review of no agent", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.review().is_some_and(|review| review.id == none)
        })
    });

    // The review of no agent is attached, and the offer is made for it.
    cx.update(|window, cx| stage.focus.focus(window, cx));
    cx.simulate_keystrokes("a");
    let asked = stage.keeper.expect(
        Request::AttachReview {
            review: none.clone(),
            thread: None,
        },
        &mut cx,
    );
    stage.keeper.reply(
        asked,
        Ok(Reply::AttachedReview {
            start: vec![claude()],
        }),
    );
    wait_until(&mut cx, "the offer", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.offer_to_start() == [claude()])
    });

    // The next review, which is attached too: the offer is not made for it, and it does not
    // come back with the review it was made for.
    cx.simulate_keystrokes("shift-a");
    assert!(is_drawn("review-attached:review", &mut cx));
    assert!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.offer_to_start())
            .is_empty()
    );
    assert!(!is_drawn("review-offer-start", &mut cx));
    cx.simulate_keystrokes("shift-a shift-a");
    assert_eq!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.review().map(|review| review.id.clone())),
        Some(none.clone())
    );
    assert!(!is_drawn("review-offer-start", &mut cx));

    // Offered again, and the workspace on screen changes: the offer goes, and is not there
    // when the workspace comes back.
    cx.simulate_keystrokes("a");
    let asked = stage.keeper.expect(
        Request::AttachReview {
            review: none,
            thread: None,
        },
        &mut cx,
    );
    stage.keeper.reply(
        asked,
        Ok(Reply::AttachedReview {
            start: vec![claude()],
        }),
    );
    wait_until(&mut cx, "the offer again", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| !pane.offer_to_start().is_empty())
    });
    go_to_workspace('2', &mut stage, &mut cx);
    go_to_workspace('1', &mut stage, &mut cx);
    wait_for_files(&stage.pane, &mut cx);
    wait_until(&mut cx, "the review of no agent again", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.review().is_some() && pane.reviews().is_some()
        })
    });
    assert!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.offer_to_start())
            .is_empty()
    );
    assert!(!is_drawn("review-offer-start", &mut cx));
}

//! Component test: whose review the Review pane shows, on GPUI's test platform. Keys and
//! clicks go in; the pane is read back through the model it shows. The test plays the
//! Keeper on the other end of a socket pair: it sets the agents and the reviews the
//! Keeper would hold, and the Keeper's own answers for the history and the uncommitted
//! changes come from the helper of `common`.

mod common;

use std::path::{Path, PathBuf};

use gpui_kit::{Modifiers, TestAppContext, VisualTestContext};
use yaar_link::{
    Anchor, Author, Comment, CommentId, CreditedFile, DraftOn, Fact, FileStatus, History, Lines,
    Reply, Request, ReviewId, SessionState, Side as LinkSide, Thread, ThreadId, TurnId,
};
use yaar_review_pane::ChangeShown;

use common::{
    COMMIT, Stage, agents_of, drawn, file, id, link_up, open_window, review_with, reviews_of,
    session, session_id, since_main, texts, wait_for_files, wait_until, workspace,
};

fn r1() -> ReviewId {
    ReviewId::new("1".repeat(32))
}

fn r2() -> ReviewId {
    ReviewId::new("2".repeat(32))
}

/// A thread on the changed line of `story.txt`, which only the review that holds it shows.
fn thread_on_story(digit: char) -> Thread {
    Thread {
        id: ThreadId::new(digit.to_string().repeat(32)),
        anchor: Anchor {
            tree: None,
            path: PathBuf::from("story.txt"),
            side: LinkSide::New,
            lines: Lines { first: 2, last: 2 },
            text: "c".to_owned(),
        },
        resolved: false,
        addressed: false,
        comments: vec![Comment {
            id: CommentId::new(digit.to_string().repeat(32)),
            author: Author::Reviewer,
            text: "a thought".to_owned(),
            round: Some(1),
        }],
        now: None,
        state: yaar_link::ThreadState::Sent,
    }
}

/// What the Keeper lists as uncommitted: one change no agent made.
fn outside() -> Vec<CreditedFile> {
    vec![CreditedFile {
        file: file("outside.txt", FileStatus::Added),
        by: Vec::new(),
    }]
}

/// A window on workspace `1`, which shows the changes of one file, whose Keeper holds
/// these reviews: R1 of session `a`, and R2 of session `b`, with a thread of its own.
/// No session is told yet.
fn open_with_two_reviews(cx: &mut TestAppContext) -> (Stage, VisualTestContext) {
    let (models, keeper) = link_up(vec![workspace('1', "feature")], cx);
    keeper.set_uncommitted(outside());
    keeper.set_history(History {
        workspace: id('1'),
        start: Some(COMMIT.to_owned()),
        entries: Vec::new(),
    });
    let mut second = review_with(r2(), Some(session_id('b')), None);
    second.threads.push(thread_on_story('t'));
    keeper.set_reviews(reviews_of(vec![
        review_with(r1(), Some(session_id('a')), None),
        second,
    ]));
    let (mut stage, mut cx) = open_window(models, keeper, cx);
    let changes = since_main(Vec::new(), vec![file("story.txt", FileStatus::Modified)]);
    stage
        .keeper
        .show(&changes, |_| texts("a\nb\n", "a\nc\n"), &mut cx);
    wait_for_files(&stage.pane, &mut cx);
    wait_until(&mut cx, "the reviews", |cx| {
        stage.pane.read_with(cx, |pane, _| pane.review().is_some())
    });
    (stage, cx)
}

fn session_a() -> yaar_link::Session {
    session('a', "Claude Code 1", SessionState::Works, Some(r1()))
}

fn session_b() -> yaar_link::Session {
    session('b', "Claude Code 2", SessionState::Works, Some(r2()))
}

/// Tells the Window these sessions of workspace `1` and waits until the pane holds them.
fn tell_sessions(stage: &mut Stage, sessions: Vec<yaar_link::Session>, cx: &mut VisualTestContext) {
    stage.keeper.tell_agents(agents_of(sessions.clone()));
    wait_until(cx, "the sessions in the pane", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.sessions() == sessions)
    });
}

fn review_shown(stage: &Stage, cx: &mut VisualTestContext) -> Option<ReviewId> {
    stage
        .pane
        .read_with(cx, |pane, _| pane.review().map(|review| review.id.clone()))
}

fn thread_drawn(digit: char, cx: &mut VisualTestContext) -> bool {
    drawn(
        &format!(
            "review-thread-{}",
            ThreadId::new(digit.to_string().repeat(32))
        ),
        cx,
    )
    .is_some()
}

fn click(selector: &str, cx: &mut VisualTestContext) {
    let at = drawn(selector, cx)
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
        .center();
    cx.simulate_click(at, Modifiers::none());
}

/// Waits until the Keeper has been asked for the uncommitted changes of this review.
fn wait_for_uncommitted_of(stage: &Stage, review: ReviewId, cx: &mut VisualTestContext) {
    wait_until(cx, "the uncommitted changes of the review shown", |_| {
        stage.keeper.uncommitted_review() == Some(review.clone())
    });
}

#[gpui_kit::test]
fn with_one_agent_session_the_pane_names_it_and_with_several_the_reviewer_chooses_whose_review_to_show(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with_two_reviews(cx);
    assert!(
        drawn("review-session", &mut cx).is_none(),
        "no session was told: none is named"
    );

    // One session: the pane names it above its review.
    tell_sessions(&mut stage, vec![session_a()], &mut cx);
    assert!(drawn("review-session", &mut cx).is_some());
    assert!(drawn("review-session-0", &mut cx).is_none());
    // What is drawn is the title of the session, not the name of its agent.
    assert!(
        drawn("review-session-name-0:Claude Code 1", &mut cx).is_some(),
        "the session is named by its title"
    );
    assert_eq!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.review_session().cloned()),
        Some(session_id('a'))
    );
    assert_eq!(review_shown(&stage, &mut cx), Some(r1()));
    assert!(!thread_drawn('t', &mut cx));

    // Several: each is named, and a click shows the review of that session.
    tell_sessions(&mut stage, vec![session_a(), session_b()], &mut cx);
    assert!(drawn("review-session", &mut cx).is_none());
    assert!(drawn("review-session-0", &mut cx).is_some());
    assert!(drawn("review-session-1", &mut cx).is_some());
    assert!(drawn("review-session-2", &mut cx).is_none());
    assert!(drawn("review-session-name-0:Claude Code 1", &mut cx).is_some());
    assert!(drawn("review-session-name-1:Claude Code 2", &mut cx).is_some());
    assert_eq!(review_shown(&stage, &mut cx), Some(r1()));
    click("review-session-1", &mut cx);
    assert_eq!(review_shown(&stage, &mut cx), Some(r2()));
    assert_eq!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.review_session().cloned()),
        Some(session_id('b'))
    );
    assert!(thread_drawn('t', &mut cx), "the threads of R2 are drawn");
    // The uncommitted changes are those of the review shown.
    wait_for_uncommitted_of(&stage, r2(), &mut cx);

    // The next review, and from the last the first.
    cx.simulate_keystrokes("shift-a");
    assert_eq!(review_shown(&stage, &mut cx), Some(r1()));
    assert!(!thread_drawn('t', &mut cx), "the threads of R2 are gone");
    wait_for_uncommitted_of(&stage, r1(), &mut cx);
    cx.simulate_keystrokes("shift-a");
    assert_eq!(review_shown(&stage, &mut cx), Some(r2()));
    wait_for_uncommitted_of(&stage, r2(), &mut cx);
}

/// The width of what is drawn for a session's name, which on the test platform is the
/// number of its characters times the width of one.
fn name_width(index: usize, title: &str, cx: &mut VisualTestContext) -> f32 {
    let selector = format!("review-session-name-{index}:{title}");
    let bounds = drawn(&selector, cx).unwrap_or_else(|| panic!("{selector} is not drawn"));
    f32::from(bounds.size.width)
}

#[gpui_kit::test]
fn the_text_drawn_for_a_session_is_its_title_and_not_the_name_of_its_agent(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with_two_reviews(cx);
    let short = "Short";
    // No spaces: the test platform measures a space differently from a letter.
    let long = "LongerTitleOfTheSession";
    let titled = |title: &str| session('a', title, SessionState::Works, Some(r1()));

    // The selector says what the title is; the width of what is drawn says what text is
    // in it, since the agent's name has one length whatever the title is. A width is
    // rounded to a pixel, hence the margin.
    tell_sessions(&mut stage, vec![titled(short)], &mut cx);
    let one_short = name_width(0, short, &mut cx);
    tell_sessions(&mut stage, vec![titled(long)], &mut cx);
    let one_long = name_width(0, long, &mut cx);
    assert!(one_short > 0.0, "the title is drawn with a width");
    let chars = long.chars().count() as f32 / short.chars().count() as f32;
    assert!(
        (one_long / one_short - chars).abs() < 0.1,
        "one session: the text drawn is {chars} times as wide for a title that long: \
         {one_short} and {one_long}"
    );

    // With several, each is drawn with its own title.
    let other = session('b', short, SessionState::Works, Some(r2()));
    tell_sessions(&mut stage, vec![titled(long), other], &mut cx);
    let first = name_width(0, long, &mut cx);
    let second = name_width(1, short, &mut cx);
    assert!(
        (first / second - chars).abs() < 0.1,
        "several sessions: the text drawn is {chars} times as wide for a title that long: \
         {second} and {first}"
    );
}

#[gpui_kit::test]
fn a_comment_on_a_change_no_agent_made_goes_to_the_review_on_screen(cx: &mut TestAppContext) {
    let (mut stage, mut cx) = open_with_two_reviews(cx);
    tell_sessions(&mut stage, vec![session_a(), session_b()], &mut cx);
    // The Keeper answers R2's question with a list of its own, so that the pane showing it
    // says the answer came: a list read again after the click would ask for the file twice.
    wait_for_uncommitted_of(&stage, r1(), &mut cx);
    let mut listed = outside();
    listed[0].file = file("outside.txt", FileStatus::Modified);
    stage.keeper.set_uncommitted(listed.clone());
    click("review-session-1", &mut cx);
    assert_eq!(review_shown(&stage, &mut cx), Some(r2()));

    // The change no agent made is in the list of every review; its file is read.
    wait_for_uncommitted_of(&stage, r2(), &mut cx);
    wait_until(&mut cx, "the uncommitted list of R2", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.uncommitted() == Some(&listed[..]))
    });
    click("review-history-uncommitted", &mut cx);
    let asked = stage.keeper.expect(
        Request::ReadVersions {
            workspace: id('1'),
            old: Some(COMMIT.to_owned()),
            old_path: PathBuf::from("outside.txt"),
            new: None,
            path: PathBuf::from("outside.txt"),
        },
        &mut cx,
    );
    stage
        .keeper
        .reply(asked, texts("", "new\nlines\n").map(Reply::File));
    wait_until(&mut cx, "the file of the change no agent made", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.change_shown() == ChangeShown::Uncommitted
                && pane.made_by_no_agent(&PathBuf::from("outside.txt"))
                && pane
                    .position()
                    .is_some_and(|at| at.path.as_path() == Path::new("outside.txt"))
        })
    });

    // A comment on a line of it is saved in R2, the review on screen.
    cx.update(|window, cx| stage.focus.focus(window, cx));
    cx.simulate_keystrokes("c");
    let opened = stage
        .pane
        .read_with(&cx, |pane, _| pane.comment_box())
        .expect("a comment box opens on the line of the change no agent made");
    assert_eq!(opened.review, r2());
    cx.simulate_input("x");
    let (_, saved) = stage.keeper.request(&mut cx);
    let Request::SaveDraft {
        review,
        thread,
        comment,
        on: DraftOn::NewThread { anchor },
        text,
    } = saved
    else {
        panic!("a draft was to be saved, and {saved:?} came");
    };
    assert_eq!(review, r2());
    assert_eq!(
        (thread, comment, text),
        (
            opened.thread.clone(),
            opened.comment.clone(),
            "x".to_owned()
        )
    );
    assert_eq!(anchor.path, PathBuf::from("outside.txt"));

    // With R1 on screen the same line takes its comment to R1.
    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes("shift-a");
    assert_eq!(review_shown(&stage, &mut cx), Some(r1()));
    cx.simulate_keystrokes("c");
    let opened = stage
        .pane
        .read_with(&cx, |pane, _| pane.comment_box())
        .expect("a comment box opens");
    assert_eq!(opened.review, r1());
}

#[gpui_kit::test]
fn the_review_shown_first_is_that_of_no_agent_and_else_that_of_the_first_session_that_runs(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with_two_reviews(cx);
    // The session that started first ended; the second one runs: its review is shown.
    let mut ended = session_a();
    ended.state = SessionState::NotRunning;
    tell_sessions(&mut stage, vec![ended, session_b()], &mut cx);
    assert_eq!(review_shown(&stage, &mut cx), Some(r2()));
    assert_eq!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.review_session().cloned()),
        Some(session_id('b'))
    );

    // A session that holds no review is not named: its review went to the next session.
    let mut taken = session_b();
    taken.review = None;
    stage
        .keeper
        .tell_agents(agents_of(vec![session_a(), taken]));
    wait_until(&mut cx, "the session that holds no review gone", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.sessions().len() == 1)
    });
    assert!(drawn("review-session", &mut cx).is_some());
    assert!(drawn("review-session-0", &mut cx).is_none());
}

#[gpui_kit::test]
fn the_review_of_no_agent_is_shown_while_it_exists_and_names_no_session(cx: &mut TestAppContext) {
    let (models, keeper) = link_up(vec![workspace('1', "feature")], cx);
    let (mut stage, mut cx) = open_window(models, keeper, cx);
    let changes = since_main(Vec::new(), vec![file("story.txt", FileStatus::Modified)]);
    stage
        .keeper
        .show(&changes, |_| texts("a\n", "b\n"), &mut cx);
    wait_for_files(&stage.pane, &mut cx);
    wait_until(&mut cx, "the review of no agent", |cx| {
        stage.pane.read_with(cx, |pane, _| pane.review().is_some())
    });
    assert_eq!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.review_session().cloned()),
        None
    );
    assert!(drawn("review-session", &mut cx).is_none());
    // The next review, with no session to go to, says so and changes nothing.
    cx.update(|window, cx| stage.focus.focus(window, cx));
    cx.simulate_keystrokes("shift-a");
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.notice()),
        Some("There is no other review to show".to_owned())
    );
}

#[gpui_kit::test]
fn a_turn_end_shows_the_review_of_its_session_unless_the_selection_is_kept(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with_two_reviews(cx);
    tell_sessions(&mut stage, vec![session_a(), session_b()], &mut cx);
    assert_eq!(review_shown(&stage, &mut cx), Some(r1()));
    let turn_of_b = |state: &str| Fact::TurnEnded {
        workspace: id('1'),
        session: session_id('b'),
        turn: TurnId::new("c".repeat(32)),
        state: Some(state.repeat(40)),
        since: Some("5".repeat(40)),
    };

    // Kept: the review stays, the offer holds it, and taking the offer shows it.
    cx.update(|window, cx| stage.focus.focus(window, cx));
    cx.simulate_keystrokes("shift-t");
    stage.keeper.tell(turn_of_b("6"));
    wait_until(&mut cx, "the offer", |cx| {
        stage.pane.read_with(cx, |pane, _| pane.offer().is_some())
    });
    assert_eq!(review_shown(&stage, &mut cx), Some(r1()));
    cx.simulate_keystrokes("shift-o");
    assert_eq!(review_shown(&stage, &mut cx), Some(r2()));
    wait_for_uncommitted_of(&stage, r2(), &mut cx);
    // The comparison of the offer is the next request, made for the review of the session
    // that finished; the test does not answer it.
    let (_, asked) = stage.keeper.request(&mut cx);
    assert!(
        matches!(&asked, Request::CompareForReview { review, .. } if *review == r2()),
        "{asked:?}"
    );

    // Not kept: the turn end shows the review of its session at once.
    cx.simulate_keystrokes("shift-a");
    assert_eq!(review_shown(&stage, &mut cx), Some(r1()));
    cx.simulate_keystrokes("shift-t");
    stage.keeper.tell(turn_of_b("7"));
    wait_until(&mut cx, "the review of the session that finished", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.review_session() == Some(&session_id('b'))
        })
    });
    wait_for_uncommitted_of(&stage, r2(), &mut cx);
}

#[gpui_kit::test]
fn a_reviewer_who_moves_to_another_review_leaves_the_open_comment_box_and_a_turn_end_leaves_it_open(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open_with_two_reviews(cx);
    tell_sessions(&mut stage, vec![session_a(), session_b()], &mut cx);
    let box_of = |stage: &Stage, cx: &mut VisualTestContext| {
        stage.pane.read_with(cx, |pane, _| pane.comment_box())
    };

    // A box on R1 is closed by a click on the session of R2, and what was typed in it
    // stays where the Keeper was told to keep it.
    cx.update(|window, cx| stage.focus.focus(window, cx));
    cx.simulate_keystrokes("c");
    assert_eq!(box_of(&stage, &mut cx).map(|open| open.review), Some(r1()));
    cx.simulate_input("x");
    let (_, saved) = stage.keeper.request(&mut cx);
    assert!(
        matches!(&saved, Request::SaveDraft { review, text, .. } if *review == r1() && text == "x"),
        "{saved:?}"
    );
    click("review-session-1", &mut cx);
    assert_eq!(review_shown(&stage, &mut cx), Some(r2()));
    assert_eq!(box_of(&stage, &mut cx), None);

    // A box on R2 stays open when a turn end shows the review of the other session: the
    // reviewer is typing, and a turn end has no business with the keys.
    cx.update(|window, cx| stage.focus.focus(window, cx));
    cx.simulate_keystrokes("c");
    assert_eq!(box_of(&stage, &mut cx).map(|open| open.review), Some(r2()));
    stage.keeper.tell(Fact::TurnEnded {
        workspace: id('1'),
        session: session_id('a'),
        turn: TurnId::new("c".repeat(32)),
        state: Some("6".repeat(40)),
        since: Some("5".repeat(40)),
    });
    wait_until(&mut cx, "the review of the session that finished", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.review_session() == Some(&session_id('a'))
        })
    });
    assert_eq!(box_of(&stage, &mut cx).map(|open| open.review), Some(r2()));
}

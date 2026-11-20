//! The agent's replies, and what only the reviewer does to a thread, through the engine
//! with the real Store and the real Recorder beneath, on a repository the test makes.
//!
//! A round is sent here by copying the review as text: the one send that is built under
//! this crate's calls for replies.

mod support;

use std::path::PathBuf;

use support::{Scratch, commit_all, engine, id, init, new_thread, open, session, store, write};
use yaar_recorder::Worktree;
use yaar_review_engine::{
    Author, CommentId, DraftOn, DraftSave, EngineError, ReviewError, ReviewId, Reviews, SessionId,
    ThreadId, ThreadReply, ThreadState,
};
use yaar_store::Store;

// A repository with one file of ten lines, and its worktree.
fn repo(scratch: &Scratch) -> (PathBuf, Worktree) {
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\ntwo\nthree\nfour\nfive\nsix\n");
    commit_all(&repo, "first");
    let worktree = open(&repo);
    (repo, worktree)
}

fn thread(n: u128) -> ThreadId {
    ThreadId::new(id(n))
}

// A reply to thread `n` as the reply tool gave it.
fn reply(n: u128, text: &str, addressed: bool) -> ThreadReply {
    ThreadReply {
        thread: thread(n),
        text: text.to_string(),
        addressed,
    }
}

// Starts session `n`, whose review is made, and gives the review.
fn started(reviews: &Reviews, store: &Store, n: u128) -> (SessionId, ReviewId) {
    let record = session(store, n);
    let review = reviews.session_started(&record).expect("start the session");
    (record.id, review)
}

// Writes a draft on each thread of `threads` (thread `n` on line `n` of `a.txt`) and sends
// the review as a copy: the next round.
fn send_threads(
    reviews: &Reviews,
    worktree: &Worktree,
    review: &ReviewId,
    threads: std::ops::RangeInclusive<u128>,
) {
    let lines = ["one", "two", "three", "four", "five", "six"];
    for n in threads {
        let line = u32::try_from(n).expect("a small number");
        let save = new_thread(
            n,
            "a.txt",
            line,
            lines[usize::try_from(n - 1).unwrap()],
            "why?",
        );
        reviews
            .save_draft(Some(worktree), review, &save)
            .expect("save the draft");
    }
    reviews.copy_as_text(worktree, review).expect("copy");
}

// The Store and the engine opened anew on the same file, as after a restart.
fn reopened(scratch: &Scratch, store: Store, reviews: Reviews) -> (Store, Reviews) {
    drop(reviews);
    drop(store);
    let store = self::store(scratch);
    let reviews = engine(scratch, store.clone());
    (store, reviews)
}

#[test]
fn a_reply_is_kept_byte_for_byte_as_the_agents_comment_in_the_newest_round() {
    let scratch = Scratch::new();
    let (_, worktree) = repo(&scratch);
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let (a, review) = started(&reviews, &store, 1);

    // Round 1 sends thread 1; a follow-up of the reviewer sends round 2.
    send_threads(&reviews, &worktree, &review, 1..=1);
    reviews
        .save_draft(
            Some(&worktree),
            &review,
            &DraftSave {
                thread: thread(1),
                comment: CommentId::new(id(2001)),
                on: DraftOn::Thread,
                text: "and the next line?".to_string(),
            },
        )
        .expect("a follow-up");
    reviews
        .copy_as_text(&worktree, &review)
        .expect("copy the follow-up");

    let hostile = "say \"no\" and 'yes'\n`rm -rf $(pwd)`; echo $HOME && true\r\n\ttab \\ é 🦀\n";
    let (found, comment) = reviews
        .agent_replied(&a, &reply(1, hostile, true))
        .expect("the reply is kept");
    assert_eq!(found, review);

    let (_, reviews) = reopened(&scratch, store, reviews);
    let read = reviews.review(&review).expect("read the review");
    let (held, kept) = read.comment(&comment).expect("the reply is read back");
    assert_eq!(held.id, thread(1));
    assert_eq!(kept.text, hostile);
    assert_eq!(kept.author, Author::Agent);
    assert_eq!(kept.round, Some(2));
    assert!(held.addressed);
    assert!(!held.resolved);
    assert_eq!(read.state_of(&thread(1)), Some(ThreadState::Addressed));
    // The reply is the last comment, after the reviewer's follow-up.
    assert_eq!(held.comments.last().map(|c| &c.id), Some(&comment));
}

#[test]
fn a_reply_without_the_mark_takes_an_earlier_mark_away_and_each_reply_draws_its_own_id() {
    let scratch = Scratch::new();
    let (_, worktree) = repo(&scratch);
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let (a, review) = started(&reviews, &store, 1);
    send_threads(&reviews, &worktree, &review, 1..=1);

    let (_, first) = reviews
        .agent_replied(&a, &reply(1, "fixed", true))
        .expect("reply");
    let (_, second) = reviews
        .agent_replied(&a, &reply(1, "no, wait", false))
        .expect("reply");
    assert_ne!(first, second);

    let (_, reviews) = reopened(&scratch, store, reviews);
    let read = reviews.review(&review).expect("read the review");
    assert_eq!(
        read.state_of(&thread(1)),
        Some(ThreadState::WaitsForReviewer)
    );
    assert_eq!(read.thread(&thread(1)).expect("held").comments.len(), 3);
}

#[test]
fn a_reply_to_a_thread_of_another_sessions_review_is_refused_and_keeps_nothing() {
    let scratch = Scratch::new();
    let (_, worktree) = repo(&scratch);
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let (a, review_a) = started(&reviews, &store, 1);
    let (b, review_b) = started(&reviews, &store, 2);
    assert_ne!(review_a, review_b);
    send_threads(&reviews, &worktree, &review_a, 1..=1);
    send_threads(&reviews, &worktree, &review_b, 2..=2);
    let before = (
        reviews.review(&review_a).expect("read"),
        reviews.review(&review_b).expect("read"),
    );

    // A names B's thread, and B names A's.
    assert!(matches!(
        reviews.agent_replied(&a, &reply(2, "mine", true)),
        Err(EngineError::Refused(ReviewError::UnknownThread(t))) if t == thread(2)
    ));
    assert!(matches!(
        reviews.agent_replied(&b, &reply(1, "mine", true)),
        Err(EngineError::Refused(ReviewError::UnknownThread(t))) if t == thread(1)
    ));
    // A thread that is nowhere.
    assert!(matches!(
        reviews.agent_replied(&a, &reply(9, "mine", true)),
        Err(EngineError::Refused(ReviewError::UnknownThread(_)))
    ));

    let (_, reviews) = reopened(&scratch, store, reviews);
    assert_eq!(reviews.review(&review_a).expect("read"), before.0);
    assert_eq!(reviews.review(&review_b).expect("read"), before.1);
}

#[test]
fn a_reply_from_a_session_with_no_review_or_before_any_send_is_refused() {
    let scratch = Scratch::new();
    let (_, worktree) = repo(&scratch);
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let (a, review) = started(&reviews, &store, 1);
    reviews
        .save_draft(
            Some(&worktree),
            &review,
            &new_thread(1, "a.txt", 1, "one", "why?"),
        )
        .expect("save the draft");

    // Nothing was sent: there is no round for the reply.
    assert!(matches!(
        reviews.agent_replied(&a, &reply(1, "x", true)),
        Err(EngineError::Refused(ReviewError::NothingSent(t))) if t == thread(1)
    ));
    // A session the Store does not hold.
    let unknown = SessionId::new(id(77));
    assert!(matches!(
        reviews.agent_replied(&unknown, &reply(1, "x", true)),
        Err(EngineError::NoReviewOfSession(s)) if s == unknown
    ));
    assert_eq!(
        reviews
            .review(&review)
            .expect("read")
            .thread(&thread(1))
            .expect("held")
            .comments
            .len(),
        1
    );
}

#[test]
fn the_reviewer_resolves_and_reopens_a_thread_and_the_engine_keeps_it() {
    let scratch = Scratch::new();
    let (_, worktree) = repo(&scratch);
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let (a, review) = started(&reviews, &store, 1);
    send_threads(&reviews, &worktree, &review, 1..=2);
    reviews
        .agent_replied(&a, &reply(1, "done", true))
        .expect("reply");

    reviews.resolve(&review, &thread(1)).expect("resolve");
    reviews.resolve(&review, &thread(2)).expect("resolve");
    // A reply to a resolved thread leaves it resolved.
    reviews
        .agent_replied(&a, &reply(1, "also this", true))
        .expect("reply");
    let (store, reviews) = reopened(&scratch, store, reviews);
    let read = reviews.review(&review).expect("read");
    assert_eq!(read.state_of(&thread(1)), Some(ThreadState::Resolved));
    assert_eq!(read.state_of(&thread(2)), Some(ThreadState::Resolved));

    reviews.reopen(&review, &thread(1)).expect("reopen");
    let (_, reviews) = reopened(&scratch, store, reviews);
    let read = reviews.review(&review).expect("read");
    assert_eq!(read.state_of(&thread(1)), Some(ThreadState::Addressed));
    assert_eq!(read.state_of(&thread(2)), Some(ThreadState::Resolved));
}

#[test]
fn a_thread_that_is_not_the_reviews_is_not_resolved_or_reopened() {
    let scratch = Scratch::new();
    let (_, worktree) = repo(&scratch);
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let (_, review_a) = started(&reviews, &store, 1);
    let (_, review_b) = started(&reviews, &store, 2);
    send_threads(&reviews, &worktree, &review_a, 1..=1);

    for result in [
        reviews.resolve(&review_b, &thread(1)),
        reviews.reopen(&review_b, &thread(1)),
    ] {
        assert!(matches!(
            result,
            Err(EngineError::Refused(ReviewError::UnknownThread(t))) if t == thread(1)
        ));
    }
    let unknown = ReviewId::new(id(88));
    assert!(matches!(
        reviews.resolve(&unknown, &thread(1)),
        Err(EngineError::UnknownReview(r)) if r == unknown
    ));
    let read = reviews.review(&review_a).expect("read");
    assert_eq!(read.state_of(&thread(1)), Some(ThreadState::Sent));
}

#[test]
fn one_call_resolves_every_addressed_thread_and_the_engine_keeps_them() {
    let scratch = Scratch::new();
    let (_, worktree) = repo(&scratch);
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let (a, review) = started(&reviews, &store, 1);
    send_threads(&reviews, &worktree, &review, 1..=4);
    reviews
        .agent_replied(&a, &reply(1, "done", true))
        .expect("reply");
    reviews
        .agent_replied(&a, &reply(2, "done", true))
        .expect("reply");
    reviews
        .agent_replied(&a, &reply(3, "why?", false))
        .expect("reply");

    assert_eq!(
        reviews.resolve_addressed(&review).expect("resolve"),
        vec![thread(1), thread(2)]
    );
    assert!(
        reviews
            .resolve_addressed(&review)
            .expect("resolve again")
            .is_empty()
    );

    let (_, reviews) = reopened(&scratch, store, reviews);
    let read = reviews.review(&review).expect("read");
    let states: Vec<_> = (1..=4).map(|n| read.state_of(&thread(n))).collect();
    assert_eq!(
        states,
        [
            Some(ThreadState::Resolved),
            Some(ThreadState::Resolved),
            Some(ThreadState::WaitsForReviewer),
            Some(ThreadState::Sent),
        ]
    );
}

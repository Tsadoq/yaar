//! Attaching a review to the user's next message, detaching it, and the session a CLI
//! started for a review, through the engine with the real Store beneath.

mod support;

use std::path::PathBuf;

use support::{Scratch, engine, id, session, session_in, store, workspace};
use yaar_review_engine::{
    Attachment, CommentId, DraftOn, DraftSave, EngineError, NewAnchor, Offer, ReviewError,
    ReviewId, Reviews, Scope, SessionNow, Side, ThreadId,
};
use yaar_store::{AttachedRecord, EndedBy, Store, WorkspaceId};

// A save that starts thread `n` on a version the test names, so that no worktree is needed.
fn draft(n: u128, text: &str) -> DraftSave {
    DraftSave {
        thread: ThreadId::new(id(n)),
        comment: CommentId::new(id(n + 1000)),
        on: DraftOn::NewThread(NewAnchor {
            tree: Some(id(0xface)),
            path: PathBuf::from("a.txt"),
            side: Side::New,
            first: 1,
            last: 1,
            text: "one".to_owned(),
        }),
        text: text.to_owned(),
    }
}

fn running(of: &yaar_store::SessionRecord) -> SessionNow {
    SessionNow {
        id: of.id.clone(),
        cli: of.agent.clone(),
        running: true,
        waits_for_permission: false,
    }
}

// The review of a session that started, with a draft on thread 1.
fn review_with_a_draft(reviews: &Reviews, store: &Store) -> (yaar_store::SessionRecord, ReviewId) {
    let started = session(store, 1);
    let review = reviews.session_started(&started).expect("start");
    reviews
        .save_draft(None, &review, &draft(1, "why one"))
        .expect("a draft");
    (started, review)
}

fn attached_in(store: &Store, review: &ReviewId) -> Option<AttachedRecord> {
    store
        .review(review)
        .expect("read")
        .expect("the review is kept")
        .attached
}

fn agent_of(store: &Store, review: &ReviewId) -> Option<String> {
    store
        .review(review)
        .expect("read")
        .expect("the review is kept")
        .agent
}

#[test]
fn an_attachment_is_kept_and_a_second_engine_on_the_same_file_reads_it() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let (started, review) = review_with_a_draft(&reviews, &store);

    let offer = reviews
        .attach(&review, Attachment::Review, &[running(&started)])
        .expect("attach");
    assert_eq!(offer, Offer::Nothing);

    // As after a restart: the file opened anew, a new engine over it.
    drop(reviews);
    drop(store);
    let store = self::store(&scratch);
    let reviews = engine(&scratch, store.clone());
    assert_eq!(
        reviews.review(&review).expect("read").attached(),
        Some(&Attachment::Review)
    );
    assert_eq!(attached_in(&store, &review), Some(AttachedRecord::Review));

    // One thread alone replaces it, and is kept as that.
    reviews
        .attach(
            &review,
            Attachment::Thread(ThreadId::new(id(1))),
            &[running(&started)],
        )
        .expect("attach one thread");
    assert_eq!(
        reviews.review(&review).expect("read").attached(),
        Some(&Attachment::Thread(ThreadId::new(id(1))))
    );
}

#[test]
fn attaching_gives_the_offer_to_start_an_agent_that_does_not_run_whatever_else_runs() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let (started, review) = review_with_a_draft(&reviews, &store);
    let other = session(&store, 2);
    store.set_ended(&started.id, EndedBy::Cli).expect("end");

    let sessions = [
        SessionNow {
            running: false,
            ..running(&started)
        },
        running(&other),
    ];
    let offer = reviews
        .attach(&review, Attachment::Review, &sessions)
        .expect("attach");

    assert_eq!(
        offer,
        Offer::Start {
            cli: "claude".to_owned()
        }
    );
    // Offered and attached: the review rides with the first message of the agent started.
    assert_eq!(attached_in(&store, &review), Some(AttachedRecord::Review));
}

#[test]
fn what_cannot_be_attached_is_refused_with_nothing_kept_and_what_was_attached_stays() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let (started, review) = review_with_a_draft(&reviews, &store);
    let sessions = [running(&started)];

    // A review that is not kept.
    let unknown = ReviewId::new(id(0xdead));
    assert!(matches!(
        reviews.attach(&unknown, Attachment::Review, &sessions),
        Err(EngineError::UnknownReview(found)) if found == unknown
    ));
    // A thread the review does not hold.
    assert!(matches!(
        reviews.attach(&review, Attachment::Thread(ThreadId::new(id(9))), &sessions),
        Err(EngineError::Refused(ReviewError::UnknownThread(_)))
    ));
    assert_eq!(attached_in(&store, &review), None);

    // A review whose only thread is resolved carries nothing.
    reviews
        .attach(&review, Attachment::Review, &sessions)
        .expect("attach");
    store
        .set_resolved(&ThreadId::new(id(1)), true)
        .expect("resolve");
    assert!(matches!(
        reviews.attach(&review, Attachment::Thread(ThreadId::new(id(1))), &sessions),
        Err(EngineError::Refused(ReviewError::NothingToSend))
    ));
    assert_eq!(attached_in(&store, &review), Some(AttachedRecord::Review));
}

#[test]
fn a_detach_takes_the_attachment_away_and_keeps_every_draft() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let (started, review) = review_with_a_draft(&reviews, &store);
    reviews
        .attach(&review, Attachment::Review, &[running(&started)])
        .expect("attach");

    reviews.detach(&review).expect("detach");

    assert_eq!(attached_in(&store, &review), None);
    let kept = reviews.review(&review).expect("read");
    assert_eq!(kept.attached(), None);
    let (_, comment) = kept
        .comment(&CommentId::new(id(1001)))
        .expect("the draft is kept");
    assert_eq!((comment.round, comment.text.as_str()), (None, "why one"));
    assert_eq!(kept.unsent(Scope::Everything).len(), 1);
    assert!(kept.unsent(Scope::Attached).is_empty());

    // Nothing attached is no error, and a review that is not kept is one.
    reviews.detach(&review).expect("detach again");
    assert!(matches!(
        reviews.detach(&ReviewId::new(id(0xdead))),
        Err(EngineError::UnknownReview(_))
    ));
}

// The review of a session `n` that ended in the workspace of the tests, with a draft.
fn review_of_an_ended_session(
    reviews: &Reviews,
    store: &Store,
    n: u128,
) -> (yaar_store::SessionRecord, ReviewId) {
    let ended = session(store, n);
    let review = reviews.session_started(&ended).expect("start");
    store.set_ended(&ended.id, EndedBy::Cli).expect("end");
    (ended, review)
}

#[test]
fn a_session_started_for_a_review_takes_it_before_the_review_of_no_agent_and_leaves_what_is_attached()
 {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    reviews.of_workspace(&workspace(), None).expect("read");
    // The first session takes the workspace's review of no agent and ends.
    let (first, review) = review_of_an_ended_session(&reviews, &store, 1);
    reviews
        .save_draft(None, &review, &draft(1, "why one"))
        .expect("a draft");
    reviews
        .attach(
            &review,
            Attachment::Review,
            &[SessionNow {
                running: false,
                ..running(&first)
            }],
        )
        .expect("attach");
    // Another review of no agent, which a session that starts with no terminal opened for a
    // review would take first.
    let free = ReviewId::new(id(0xf00d));
    store
        .put_review(yaar_review_engine::Review::new(free.clone(), workspace(), None).record())
        .expect("a review of no agent");
    let third = session(&store, 3);

    let given = reviews
        .session_started_for(&third, &review)
        .expect("start for the review");

    assert_eq!(given, review);
    assert_eq!(
        agent_of(&store, &review),
        Some(third.id.as_str().to_owned())
    );
    assert_eq!(agent_of(&store, &free), None);
    // What was attached stays attached: it rides with the new session's first message.
    assert_eq!(attached_in(&store, &review), Some(AttachedRecord::Review));

    // Without the terminal's review, the same session takes the review of no agent.
    let fourth = session(&store, 4);
    store.set_ended(&third.id, EndedBy::Cli).expect("end");
    assert_eq!(reviews.session_started(&fourth).expect("start"), free);
}

#[test]
fn a_session_started_for_the_review_of_no_agent_takes_it() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let none = reviews
        .of_workspace(&workspace(), None)
        .expect("read")
        .reviews[0]
        .id()
        .clone();
    reviews
        .save_draft(None, &none, &draft(1, "no one"))
        .expect("a draft");
    reviews
        .attach(&none, Attachment::Review, &[])
        .expect("attach");
    let started = session(&store, 1);

    let given = reviews
        .session_started_for(&started, &none)
        .expect("start for the review");

    assert_eq!(given, none);
    assert_eq!(
        agent_of(&store, &none),
        Some(started.id.as_str().to_owned())
    );
    assert_eq!(attached_in(&store, &none), Some(AttachedRecord::Review));
}

#[test]
fn a_session_started_for_a_review_that_cannot_be_taken_gets_its_own_as_any_session_does() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    reviews.of_workspace(&workspace(), None).expect("read");

    // Its session runs: not free to take.
    let (running_session, review) = {
        let held = session(&store, 1);
        let review = reviews.session_started(&held).expect("start");
        (held, review)
    };
    let next = session(&store, 2);
    let given = reviews
        .session_started_for(&next, &review)
        .expect("start for the review");
    assert_ne!(given, review);
    assert_eq!(
        agent_of(&store, &review),
        Some(running_session.id.as_str().to_owned())
    );
    assert_eq!(agent_of(&store, &given), Some(next.id.as_str().to_owned()));

    // Its session ended, of another CLI: not taken by this one.
    store
        .set_ended(&running_session.id, EndedBy::Cli)
        .expect("end");
    let of_pi = yaar_store::SessionRecord {
        id: yaar_review_engine::SessionId::new(id(3)),
        workspace: workspace(),
        agent: "pi".to_owned(),
        cli_session: "cli-3".to_owned(),
        terminal: Some("t3".to_owned()),
        number: 1,
        running: true,
        ended: None,
        ended_by: None,
    };
    store.put_session(&of_pi).expect("keep the pi session");
    let given = reviews
        .session_started_for(&of_pi, &review)
        .expect("start for the review");
    assert_ne!(given, review);
    assert_eq!(
        agent_of(&store, &review),
        Some(running_session.id.as_str().to_owned())
    );

    // A review of another workspace is never taken.
    let elsewhere = WorkspaceId::new(id(0xbb));
    let there = session_in(&store, 4, elsewhere.clone());
    let there_review = reviews.session_started(&there).expect("start there");
    store.set_ended(&there.id, EndedBy::Cli).expect("end");
    let here = session(&store, 5);
    let given = reviews
        .session_started_for(&here, &there_review)
        .expect("start for the review");
    assert_ne!(given, there_review);
    assert_eq!(
        agent_of(&store, &there_review),
        Some(there.id.as_str().to_owned())
    );
}

#[test]
fn a_session_started_for_a_review_that_is_gone_is_given_a_review_as_any_session_is() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let started = session(&store, 1);

    let given = reviews
        .session_started_for(&started, &ReviewId::new(id(0xdead)))
        .expect("start");

    assert_eq!(
        agent_of(&store, &given),
        Some(started.id.as_str().to_owned())
    );
}

#[test]
fn a_session_that_holds_a_review_already_keeps_it_whatever_review_the_terminal_was_opened_for() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let held = session(&store, 2);
    let own = reviews.session_started(&held).expect("start");
    let (ended, free) = review_of_an_ended_session(&reviews, &store, 1);
    assert_ne!(own, free);

    let given = reviews
        .session_started_for(&held, &free)
        .expect("start for the review");

    // One session never holds two reviews: `review_of_session` could name only one.
    assert_eq!(given, own);
    assert_eq!(
        agent_of(&store, &free),
        Some(ended.id.as_str().to_owned()),
        "the ended session's review stays its own"
    );
}

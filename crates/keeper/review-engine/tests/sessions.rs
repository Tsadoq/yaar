mod support;

use std::sync::Arc;

use support::{Scratch, engine, id, new_thread, open, session, session_in, store, workspace};
use yaar_review_engine::EngineError;
use yaar_store::{EndedBy, SessionId, WorkspaceId};

fn agent_of(store: &yaar_store::Store, review: &yaar_review_engine::ReviewId) -> Option<String> {
    store
        .reviews_of(&workspace())
        .expect("the Store's reviews")
        .into_iter()
        .find(|kept| &kept.id == review)
        .expect("the review is kept")
        .agent
}

#[test]
fn the_first_session_of_a_workspace_takes_over_its_review_of_no_agent() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let none = reviews
        .of_workspace(&workspace(), None)
        .expect("read")
        .reviews[0]
        .id()
        .clone();
    let first = session(&store, 1);

    let given = reviews.session_started(&first).expect("start");

    assert_eq!(given, none);
    assert_eq!(agent_of(&store, &given), Some(first.id.as_str().to_owned()));
    assert_eq!(store.reviews_of(&workspace()).expect("reviews").len(), 1);
}

#[test]
fn two_sessions_that_run_at_once_have_a_review_each_and_a_third_gets_a_third() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    reviews.of_workspace(&workspace(), None).expect("read");

    let given: Vec<_> = (1..=3)
        .map(|n| {
            let started = session(&store, n);
            (
                started.id.clone(),
                reviews.session_started(&started).expect("start"),
            )
        })
        .collect();

    assert_ne!(given[0].1, given[1].1);
    assert_ne!(given[1].1, given[2].1);
    assert_ne!(given[0].1, given[2].1);
    for (session, review) in &given {
        assert_eq!(agent_of(&store, review), Some(session.as_str().to_owned()));
    }
    let kept = store.reviews_of(&workspace()).expect("reviews");
    assert_eq!(kept.len(), 3);
    assert!(kept.iter().all(|review| review.agent.is_some()));
}

#[test]
fn a_session_that_starts_in_a_workspace_with_no_review_gets_one_of_its_own() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let first = session(&store, 1);

    let given = reviews.session_started(&first).expect("start");

    assert_eq!(agent_of(&store, &given), Some(first.id.as_str().to_owned()));
    let view = reviews.of_workspace(&workspace(), None).expect("read");
    assert_eq!(view.reviews.len(), 1);
    assert_eq!(view.reviews[0].id(), &given);
}

#[test]
fn the_review_of_a_session_that_ended_stays_open_with_its_threads_for_the_next_session() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    support::init(&repo);
    support::write(&repo, "a.txt", "one\n");
    support::commit_all(&repo, "first");
    let worktree = open(&repo);
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    reviews.of_workspace(&workspace(), None).expect("read");

    let first = session(&store, 1);
    let review = reviews.session_started(&first).expect("start");
    reviews
        .save_draft(
            Some(&worktree),
            &review,
            &new_thread(1, "a.txt", 1, "one", "why one"),
        )
        .expect("a comment");
    store.set_ended(&first.id, EndedBy::Cli).expect("end");

    assert_eq!(
        agent_of(&store, &review),
        Some(first.id.as_str().to_owned())
    );

    let next = session(&store, 2);
    let taken = reviews.session_started(&next).expect("start again");
    assert_eq!(taken, review);
    assert_eq!(agent_of(&store, &review), Some(next.id.as_str().to_owned()));
    assert_eq!(reviews.review(&review).expect("read").threads().len(), 1);
}

#[test]
fn a_session_that_starts_again_keeps_its_review_and_changes_nothing() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    reviews.of_workspace(&workspace(), None).expect("read");
    let first = session(&store, 1);
    let second = session(&store, 2);
    let r1 = reviews.session_started(&first).expect("start");
    let r2 = reviews.session_started(&second).expect("start");
    // The first one's CLI ended and resumed in a new terminal: the Store has it running.
    store.set_ended(&first.id, EndedBy::Cli).expect("end");
    store.set_running(&first.id, Some("t9")).expect("resume");
    let before = store.reviews_of(&workspace()).expect("reviews");

    let again = reviews
        .session_started(&store.session(&first.id).expect("read").expect("kept"))
        .expect("start again");

    assert_eq!(again, r1);
    assert_ne!(again, r2);
    assert_eq!(store.reviews_of(&workspace()).expect("reviews"), before);
}

#[test]
fn the_review_of_an_ended_session_of_another_workspace_is_not_taken() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let elsewhere = WorkspaceId::new(id(0xbb));
    let there = session_in(&store, 1, elsewhere.clone());
    let review = reviews.session_started(&there).expect("start there");
    store.set_ended(&there.id, EndedBy::Cli).expect("end");

    let here = session(&store, 2);
    let given = reviews.session_started(&here).expect("start here");

    assert_ne!(given, review);
    let kept = store.reviews_of(&elsewhere).expect("reviews");
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].agent, Some(there.id.as_str().to_owned()));
}

#[test]
fn a_session_the_store_does_not_hold_cannot_take_a_review_and_changes_nothing() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    reviews.of_workspace(&workspace(), None).expect("read");
    let before = store.reviews_of(&workspace()).expect("reviews");
    let unkept = {
        let mut record = session(&store, 1);
        record.id = SessionId::new(id(77));
        record.cli_session = "cli-77".to_owned();
        record
    };

    let refused = reviews.session_started(&unkept);

    assert!(matches!(refused, Err(EngineError::Store(_))), "{refused:?}");
    assert_eq!(store.reviews_of(&workspace()).expect("reviews"), before);
}

#[test]
fn eight_sessions_that_start_at_once_each_end_up_with_a_review_of_their_own() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let reviews = Arc::new(engine(&scratch, store.clone()));
    reviews.of_workspace(&workspace(), None).expect("read");
    let started: Vec<_> = (1..=8).map(|n| session(&store, n)).collect();

    let threads: Vec<_> = started
        .into_iter()
        .map(|started| {
            let reviews = Arc::clone(&reviews);
            std::thread::spawn(move || {
                (
                    started.id.clone(),
                    reviews.session_started(&started).expect("start"),
                )
            })
        })
        .collect();
    let given: Vec<_> = threads
        .into_iter()
        .map(|t| t.join().expect("a start"))
        .collect();

    let mut ids: Vec<_> = given.iter().map(|(_, review)| review.clone()).collect();
    ids.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    ids.dedup();
    assert_eq!(ids.len(), 8, "two sessions share a review: {given:?}");
    for (session, review) in &given {
        assert_eq!(agent_of(&store, review), Some(session.as_str().to_owned()));
    }
    assert_eq!(store.reviews_of(&workspace()).expect("reviews").len(), 8);
}

//! Copying a review as text, through the engine with the real Store and the real
//! Recorder beneath, on a repository the test makes.

mod support;

use std::path::PathBuf;

use support::{
    Scratch, commit_all, engine, files_of, id, init, new_thread, open, store, workspace, write,
};
use yaar_review_engine::{CommentId, EngineError, Review, ReviewError, Scope, SentBy, ThreadId};

// REV-05
#[test]
fn copying_a_review_marks_the_copied_comments_sent_starts_a_round_and_records_the_state() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    write(&repo, "b.txt", "two\n");
    write(&repo, "c.txt", "three\n");
    commit_all(&repo, "first");
    write(&repo, "a.txt", "one changed\n");
    let worktree = open(&repo);

    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let w = workspace();
    let review = reviews
        .of_workspace(&w, Some(&worktree))
        .expect("read")
        .reviews[0]
        .id()
        .clone();

    // Drafts on two threads, and on a third that is resolved through the Store's call:
    // the engine has no resolve call yet.
    for save in [
        new_thread(1, "a.txt", 1, "one changed", "first draft to copy"),
        new_thread(2, "b.txt", 1, "two", "second draft to copy"),
        new_thread(3, "c.txt", 1, "three", "draft on a resolved thread"),
    ] {
        reviews
            .save_draft(Some(&worktree), &review, &save)
            .expect("save the draft");
    }
    store
        .set_resolved(&ThreadId::new(id(3)), true)
        .expect("resolve the third thread");

    // The worktree moves on after the drafts were written, and before the copy.
    write(&repo, "a.txt", "one changed\nand a new line\n");
    let states = reviews.states_of(&w).expect("the state store");
    let before_states = states.states().expect("list the states");
    let before = reviews.review(&review).expect("read the review");

    let copied = reviews.copy_as_text(&worktree, &review).expect("copy");

    // The text holds the two drafts and not the third.
    assert!(
        copied.text.contains("first draft to copy"),
        "{}",
        copied.text
    );
    assert!(copied.text.contains("second draft to copy"));
    assert!(!copied.text.contains("draft on a resolved thread"));
    assert_eq!(copied.round, 1);

    // Read back through a new engine over a new Store of the same file, as after a
    // restart: both copied comments are sent in round 1, the resolved thread's draft
    // is still a draft.
    drop(reviews);
    drop(store);
    let store_again = self::store(&scratch);
    let reviews_again = engine(&scratch, store_again);
    let after = reviews_again.review(&review).expect("read it again");
    let round_of = |n: u128| {
        after
            .comment(&CommentId::new(id(n + 1000)))
            .expect("the comment is kept")
            .1
            .round
    };
    assert_eq!(
        (round_of(1), round_of(2), round_of(3)),
        (Some(1), Some(1), None)
    );

    // One round, the first, sent by a copy.
    assert_eq!(after.rounds().len(), 1);
    let round = &after.rounds()[0];
    // A copy records its state before the round: it has one.
    let round_state = round.state.clone().expect("a copied round has its state");
    assert_eq!(round.number, 1);
    assert_eq!(round.sent_by, SentBy::Copied);

    // Its state is a new one, the newest of the store, and holds the worktree as it was
    // at the copy, the new line included.
    let states = reviews_again.states_of(&w).expect("the state store");
    let listed = states.states().expect("list the states");
    assert!(!before_states.iter().any(|s| s.id.as_str() == round_state));
    assert_eq!(listed.len(), before_states.len() + 1);
    assert_eq!(listed.last().expect("a state").id.as_str(), round_state);
    let held = states.paths(&round_state).expect("the files of the state");
    let on_disk = files_of(&repo);
    assert_eq!(held, on_disk.keys().cloned().collect::<Vec<PathBuf>>());
    for (path, content) in &on_disk {
        assert_eq!(
            states.content(&round_state, path).expect("read"),
            Some(content.clone()),
            "{}",
            path.display()
        );
    }
    let changed = states
        .content(&round_state, std::path::Path::new("a.txt"))
        .expect("read")
        .expect("a.txt is in the state");
    assert!(
        String::from_utf8(changed)
            .unwrap()
            .contains("and a new line")
    );

    // As a submit does: the round, the comments and the state are those the one path
    // every send takes gives.
    let mut expected = Review::from_record(before.record().clone());
    let sent = expected
        .send(Scope::Everything, &round_state, SentBy::Copied)
        .expect("send");
    assert_eq!(
        sent.comments,
        vec![CommentId::new(id(1001)), CommentId::new(id(1002))]
    );
    assert_eq!(expected.record(), after.record());

    // With nothing new to send a second copy is refused, with no second round and no new
    // state, though the worktree changed again, which a recording would show.
    write(&repo, "b.txt", "two, again\n");
    let refused = reviews_again.copy_as_text(&worktree, &review);
    assert!(
        matches!(
            refused,
            Err(EngineError::Refused(ReviewError::NothingToSend))
        ),
        "{refused:?}"
    );
    assert_eq!(states.states().expect("list").len(), listed.len());
    let still = reviews_again.review(&review).expect("read");
    assert_eq!(still.rounds().len(), 1);
}

#[test]
fn a_review_with_only_a_resolved_draft_is_refused_and_records_no_state() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "first");
    let worktree = open(&repo);

    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let w = workspace();
    let review = reviews
        .of_workspace(&w, Some(&worktree))
        .expect("read")
        .reviews[0]
        .id()
        .clone();
    reviews
        .save_draft(
            Some(&worktree),
            &review,
            &new_thread(1, "a.txt", 1, "one", "hidden"),
        )
        .expect("save");
    store
        .set_resolved(&ThreadId::new(id(1)), true)
        .expect("resolve");
    let states = reviews.states_of(&w).expect("the state store");
    let known = states.states().expect("list").len();
    write(&repo, "a.txt", "one\nmore\n");

    let refused = reviews.copy_as_text(&worktree, &review);
    assert!(matches!(
        refused,
        Err(EngineError::Refused(ReviewError::NothingToSend))
    ));
    assert_eq!(states.states().expect("list").len(), known);
    let kept = reviews.review(&review).expect("read");
    assert!(kept.rounds().is_empty());
    assert_eq!(
        kept.comment(&CommentId::new(id(1001)))
            .expect("kept")
            .1
            .round,
        None
    );
}

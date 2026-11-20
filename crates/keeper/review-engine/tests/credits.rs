//! Who is credited with each path, kept with the recorded states, and what one review shows
//! of a change between two versions, through the engine with the real Store and the real
//! Recorder beneath, on a repository the test makes.

mod support;

use std::path::{Path, PathBuf};

use support::{
    Scratch, commit_all, engine, init, new_thread, open, session, store, workspace, write,
};
use yaar_recorder::{Change, StateId, Worktree};
use yaar_review_engine::{Credited, CreditedChange, EngineError, ReviewId, Reviews, WorkspaceId};
use yaar_store::{ReviewRecord, Store};

fn credited(path: &str, by: &[&str]) -> Credited {
    Credited {
        path: PathBuf::from(path),
        by: by.iter().map(|session| (*session).to_owned()).collect(),
    }
}

fn listed(path: &str, change: Change, by: &[&str], shared: bool) -> CreditedChange {
    CreditedChange {
        path: PathBuf::from(path),
        change,
        by: by.iter().map(|session| (*session).to_owned()).collect(),
        shared,
    }
}

fn record(reviews: &Reviews, w: &WorkspaceId, worktree: &Worktree) -> String {
    reviews
        .states_of(w)
        .expect("the state store")
        .record(worktree)
        .expect("record a state")
        .id
        .as_str()
        .to_owned()
}

fn session_id(n: u128) -> String {
    support::id(n)
}

/// Two sessions with a review each, in `store`, and the review of no agent: A is session 1,
/// B session 2.
struct Reviewers {
    a: ReviewId,
    b: ReviewId,
    nobody: ReviewId,
}

fn reviewers(reviews: &Reviews, store: &Store) -> Reviewers {
    let a = reviews
        .session_started(&session(store, 1))
        .expect("start A");
    let b = reviews
        .session_started(&session(store, 2))
        .expect("start B");
    // A workspace with sessions has no review of no agent of its own; the Store can hold
    // one all the same, as one a session left when it took another over.
    let nobody = ReviewId::new(support::id(0x77));
    store
        .put_review(&ReviewRecord {
            id: nobody.clone(),
            workspace: workspace(),
            agent: None,
            reviewed: None,
            threads: vec![],
            rounds: vec![],
            attached: None,
        })
        .expect("keep the review of no agent");
    Reviewers { a, b, nobody }
}

fn repository(scratch: &Scratch, files: &[&str]) -> PathBuf {
    let repo = scratch.join("repo");
    init(&repo);
    for file in files {
        write(&repo, file, "base\n");
    }
    commit_all(&repo, "first");
    repo
}

fn touch(repo: &Path, files: &[&str]) {
    for file in files {
        write(repo, file, "changed\n");
    }
}

// REV-24
#[test]
fn a_recorded_state_keeps_who_is_credited_with_each_path_and_a_path_two_sessions_changed_shows_whole_in_both_reviews_marked()
 {
    let scratch = Scratch::new();
    let repo = repository(&scratch, &["a.txt", "b.txt", "s.txt", "h.txt"]);
    let worktree = open(&repo);
    let w = workspace();
    let (a, b) = (session_id(1), session_id(2));

    let (s0, s1) = {
        let store = store(&scratch);
        let reviews = engine(&scratch, store.clone());
        let s0 = record(&reviews, &w, &worktree);
        // A's tool wrote a, B's b, both wrote s, and h was changed by hand.
        touch(&repo, &["a.txt", "b.txt", "s.txt", "h.txt"]);
        let s1 = record(&reviews, &w, &worktree);
        reviews
            .keep_credits(
                &w,
                &s1,
                &[
                    credited("a.txt", &[&a]),
                    credited("b.txt", &[&b]),
                    credited("s.txt", &[&a, &b]),
                    credited("h.txt", &[]),
                ],
            )
            .expect("keep the credits");
        (s0, s1)
    };

    // A new Reviews on a new Store opened on the same file: what is kept outlives them.
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let r = reviewers(&reviews, &store);
    let modified =
        |path: &str, by: &[&str], shared: bool| listed(path, Change::Modified, by, shared);

    // "Who is credited with each path": A's review lists what A changed, what nobody did,
    // and the path both changed.
    assert_eq!(
        reviews.credited_change(&r.a, &s0, &s1).expect("A's change"),
        [
            modified("a.txt", &[&a], false),
            modified("h.txt", &[], false),
            modified("s.txt", &[&a, &b], true),
        ]
    );
    assert_eq!(
        reviews.credited_change(&r.b, &s0, &s1).expect("B's change"),
        [
            modified("b.txt", &[&b], false),
            modified("h.txt", &[], false),
            modified("s.txt", &[&a, &b], true),
        ]
    );
    // The review of no agent shows every path, and marks the one two sessions changed.
    assert_eq!(
        reviews
            .credited_change(&r.nobody, &s0, &s1)
            .expect("the change of no agent"),
        [
            modified("a.txt", &[&a], false),
            modified("b.txt", &[&b], false),
            modified("h.txt", &[], false),
            modified("s.txt", &[&a, &b], true),
        ]
    );
}

// REV-24
#[test]
fn a_state_recorded_at_a_send_counts_with_the_first_later_state_that_keeps_credits() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, &["c.txt"]);
    let worktree = open(&repo);
    let w = workspace();
    let (a, b) = (session_id(1), session_id(2));
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let r = reviewers(&reviews, &store);
    reviews
        .save_draft(
            Some(&worktree),
            &r.a,
            &new_thread(1, "c.txt", 1, "base", "a comment to send"),
        )
        .expect("a draft");
    let s0 = record(&reviews, &w, &worktree);

    // A's tool writes a.txt, then the reviewer sends: the copy records a state and keeps no
    // credits.
    touch(&repo, &["a.txt"]);
    reviews.copy_as_text(&worktree, &r.a).expect("copy");
    let copy = reviews
        .review(&r.a)
        .expect("read")
        .rounds()
        .last()
        .and_then(|round| round.state.clone())
        .expect("the copy's state");
    let added = |by: &[&str]| vec![listed("a.txt", Change::Added, by, false)];

    // No turn end followed: nothing names the path, so it is no agent's, in both reviews.
    assert_eq!(
        reviews.credited_change(&r.a, &s0, &copy).expect("A"),
        added(&[])
    );
    assert_eq!(
        reviews.credited_change(&r.b, &s0, &copy).expect("B"),
        added(&[])
    );

    // B's tool writes b.txt and B's turn ends, with the credits of every uncommitted path.
    touch(&repo, &["b.txt"]);
    let s2 = record(&reviews, &w, &worktree);
    reviews
        .keep_credits(
            &w,
            &s2,
            &[credited("a.txt", &[&a]), credited("b.txt", &[&b])],
        )
        .expect("keep the credits");

    // The copy's state counts with `s2`, which lies past the range: a.txt is A's.
    assert_eq!(
        reviews.credited_change(&r.a, &s0, &copy).expect("A"),
        added(&[&a])
    );
    assert!(
        reviews
            .credited_change(&r.b, &s0, &copy)
            .expect("B")
            .is_empty()
    );
}

// REV-24
#[test]
fn credits_are_the_union_of_those_kept_after_the_older_version_up_to_the_newer_one() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, &["x.txt"]);
    let worktree = open(&repo);
    let w = workspace();
    let (a, b) = (session_id(1), session_id(2));
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let r = reviewers(&reviews, &store);

    // The older version's own credits do not count: B is credited with x.txt there.
    let s0 = record(&reviews, &w, &worktree);
    reviews
        .keep_credits(&w, &s0, &[credited("x.txt", &[&b])])
        .expect("keep");
    write(&repo, "x.txt", "by A\n");
    let s1 = record(&reviews, &w, &worktree);
    reviews
        .keep_credits(&w, &s1, &[credited("x.txt", &[&a])])
        .expect("keep");
    write(&repo, "x.txt", "by B\n");
    let s2 = record(&reviews, &w, &worktree);
    reviews
        .keep_credits(&w, &s2, &[credited("x.txt", &[&b])])
        .expect("keep");

    let x = |by: &[&str]| vec![listed("x.txt", Change::Modified, by, by.len() > 1)];
    let change = |review: &ReviewId, from: &str, to: &str| {
        reviews.credited_change(review, from, to).expect("a change")
    };
    // Up to s1 only A is credited: what s2 keeps lies past the range and is not looked at.
    assert_eq!(change(&r.a, &s0, &s1), x(&[&a]));
    assert!(change(&r.b, &s0, &s1).is_empty());
    // Up to s2 both are, in the order first credited, and the path shows in both reviews.
    assert_eq!(change(&r.a, &s0, &s2), x(&[&a, &b]));
    assert_eq!(change(&r.b, &s0, &s2), x(&[&a, &b]));
    // From s1 the state s1 itself is not counted: only B.
    assert!(change(&r.a, &s1, &s2).is_empty());
    assert_eq!(change(&r.b, &s1, &s2), x(&[&b]));
}

#[test]
fn a_commit_as_the_older_version_counts_every_state_up_to_the_newer_one_and_an_unknown_review_is_refused()
 {
    let scratch = Scratch::new();
    let repo = repository(&scratch, &["x.txt"]);
    let first = StateId::new(support::git(&repo, &["rev-parse", "HEAD"]));
    let worktree = open(&repo);
    let w = workspace();
    let a = session_id(1);
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let r = reviewers(&reviews, &store);
    reviews
        .states_of(&w)
        .expect("the state store")
        .keep_commit(&worktree, &yaar_recorder::CommitId::new(first.as_str()))
        .expect("keep the commit");
    write(&repo, "x.txt", "by A\n");
    let s1 = record(&reviews, &w, &worktree);
    reviews
        .keep_credits(&w, &s1, &[credited("x.txt", &[&a])])
        .expect("keep");

    assert_eq!(
        reviews
            .credited_change(&r.a, first.as_str(), &s1)
            .expect("from a commit"),
        [listed("x.txt", Change::Modified, &[&a], false)]
    );
    assert!(matches!(
        reviews.credited_change(&ReviewId::new(support::id(99)), first.as_str(), &s1),
        Err(EngineError::UnknownReview(_))
    ));
}

#[test]
fn keeping_credits_again_for_a_state_replaces_them() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, &["x.txt"]);
    let worktree = open(&repo);
    let w = workspace();
    let (a, b) = (session_id(1), session_id(2));
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let r = reviewers(&reviews, &store);
    let s0 = record(&reviews, &w, &worktree);
    write(&repo, "x.txt", "changed\n");
    let s1 = record(&reviews, &w, &worktree);
    reviews
        .keep_credits(&w, &s1, &[credited("x.txt", &[&a])])
        .expect("keep");
    reviews
        .keep_credits(&w, &s1, &[credited("x.txt", &[&b])])
        .expect("keep again");

    assert!(
        reviews
            .credited_change(&r.a, &s0, &s1)
            .expect("A")
            .is_empty()
    );
    assert_eq!(
        reviews.credited_change(&r.b, &s0, &s1).expect("B"),
        [listed("x.txt", Change::Modified, &[&b], false)]
    );
}

// REV-24
#[test]
fn a_state_kept_with_no_credit_for_a_clean_worktree_does_not_take_those_of_the_next_turn() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, &["x.txt"]);
    let worktree = open(&repo);
    let w = workspace();
    let b = session_id(2);
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let r = reviewers(&reviews, &store);
    let s0 = record(&reviews, &w, &worktree);

    // A writes a.txt and commits it in its turn: the turn ends on a clean worktree, so
    // the credits kept are none.
    write(&repo, "a.txt", "by A\n");
    commit_all(&repo, "A's commit");
    let s1 = record(&reviews, &w, &worktree);
    reviews.keep_credits(&w, &s1, &[]).expect("keep none");

    // B changes the committed file in its turn.
    write(&repo, "a.txt", "by B\n");
    let s2 = record(&reviews, &w, &worktree);
    reviews
        .keep_credits(&w, &s2, &[credited("a.txt", &[&b])])
        .expect("keep");

    // Up to s1, a.txt is no agent's: the credits B's turn keeps at s2 are not taken
    // for it, so it is not B's and is not hidden from A.
    assert_eq!(
        reviews.credited_change(&r.a, &s0, &s1).expect("A"),
        [listed("a.txt", Change::Added, &[], false)]
    );
    assert_eq!(
        reviews.credited_change(&r.b, &s0, &s1).expect("B"),
        [listed("a.txt", Change::Added, &[], false)]
    );
    // From s1 to s2 the file is B's alone, not shared.
    assert_eq!(
        reviews.credited_change(&r.b, &s1, &s2).expect("B"),
        [listed("a.txt", Change::Modified, &[&b], false)]
    );
    assert!(
        reviews
            .credited_change(&r.a, &s1, &s2)
            .expect("A")
            .is_empty()
    );
    // Up to s2 both reviews see the file, and A's credits are none: B's alone.
    assert_eq!(
        reviews.credited_change(&r.b, &s0, &s2).expect("B"),
        [listed("a.txt", Change::Added, &[&b], false)]
    );
    assert!(
        reviews
            .credited_change(&r.a, &s0, &s2)
            .expect("A")
            .is_empty()
    );
}

// REV-24
#[test]
fn a_state_recorded_at_a_send_after_a_clean_turn_end_still_waits_for_the_next_state_with_credits() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, &["x.txt"]);
    let worktree = open(&repo);
    let w = workspace();
    let a = session_id(1);
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let r = reviewers(&reviews, &store);
    record(&reviews, &w, &worktree);

    // A turn ends clean, then A's tool writes b.txt and the reviewer sends: that state
    // keeps nothing. A's next turn end keeps the credit of b.txt.
    write(&repo, "a.txt", "by A\n");
    commit_all(&repo, "A's commit");
    let s1 = record(&reviews, &w, &worktree);
    reviews.keep_credits(&w, &s1, &[]).expect("keep none");
    write(&repo, "b.txt", "by A\n");
    let sent = record(&reviews, &w, &worktree);
    // The turn goes on with a write of its own, so its end is a state of its own, not the
    // state of the send again (the Recorder gives back the newest state of an unchanged
    // tree).
    write(&repo, "c.txt", "by A\n");
    let s3 = record(&reviews, &w, &worktree);
    assert_ne!(sent, s3, "the turn end is a state after the send");
    reviews
        .keep_credits(
            &w,
            &s3,
            &[credited("b.txt", &[&a]), credited("c.txt", &[&a])],
        )
        .expect("keep");

    assert_eq!(
        reviews.credited_change(&r.a, &s1, &sent).expect("A"),
        [listed("b.txt", Change::Added, &[&a], false)]
    );
}

// REV-24
#[test]
fn a_state_recorded_at_a_send_does_not_wait_past_a_clean_turn_end_for_the_credits_of_another_turn()
{
    let scratch = Scratch::new();
    let repo = repository(&scratch, &["x.txt"]);
    let worktree = open(&repo);
    let w = workspace();
    let b = session_id(2);
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let r = reviewers(&reviews, &store);
    let s0 = record(&reviews, &w, &worktree);

    // A's tool writes b.txt and the reviewer sends: that state keeps nothing.
    write(&repo, "b.txt", "by A\n");
    let sent = record(&reviews, &w, &worktree);
    // A's turn then writes c.txt and commits everything: it ends on a clean worktree, so
    // it keeps no credit. A commit alone leaves the tree as it was; the write before it
    // makes this a state of its own.
    write(&repo, "c.txt", "by A\n");
    commit_all(&repo, "A's commit");
    let s2 = record(&reviews, &w, &worktree);
    assert_ne!(sent, s2, "the clean turn end is a state after the send");
    reviews.keep_credits(&w, &s2, &[]).expect("keep none");
    // B's turn changes b.txt and keeps its credit.
    write(&repo, "b.txt", "by B\n");
    let s3 = record(&reviews, &w, &worktree);
    reviews
        .keep_credits(&w, &s3, &[credited("b.txt", &[&b])])
        .expect("keep");

    // The send waits only for the end of the turn it was in, which kept none: b.txt is
    // no agent's up to it, shown to both reviews, and not B's.
    let added = [listed("b.txt", Change::Added, &[], false)];
    assert_eq!(reviews.credited_change(&r.a, &s0, &sent).expect("A"), added);
    assert_eq!(reviews.credited_change(&r.b, &s0, &sent).expect("B"), added);
}

// REV-24
#[test]
fn a_session_credited_with_a_path_in_two_states_is_listed_once_and_the_path_is_not_shared() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, &["x.txt"]);
    let worktree = open(&repo);
    let w = workspace();
    let a = session_id(1);
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let r = reviewers(&reviews, &store);
    let s0 = record(&reviews, &w, &worktree);

    write(&repo, "x.txt", "first\n");
    let s1 = record(&reviews, &w, &worktree);
    reviews
        .keep_credits(&w, &s1, &[credited("x.txt", &[&a])])
        .expect("keep");
    write(&repo, "x.txt", "second\n");
    let s2 = record(&reviews, &w, &worktree);
    reviews
        .keep_credits(&w, &s2, &[credited("x.txt", &[&a])])
        .expect("keep");

    // A is credited at both states and is one session: not two, so not shared.
    assert_eq!(
        reviews.credited_change(&r.a, &s0, &s2).expect("A"),
        [listed("x.txt", Change::Modified, &[&a], false)]
    );
    assert!(
        reviews
            .credited_change(&r.b, &s0, &s2)
            .expect("B")
            .is_empty()
    );
}

// REV-24
#[test]
fn a_state_recorded_at_a_send_stops_waiting_at_a_turn_end_that_keeps_credits_inside_the_range() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, &["x.txt"]);
    let worktree = open(&repo);
    let w = workspace();
    let a = session_id(1);
    let b = session_id(2);
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let r = reviewers(&reviews, &store);
    let s0 = record(&reviews, &w, &worktree);

    // A's tool writes b.txt and the reviewer sends: that state keeps nothing.
    write(&repo, "b.txt", "by A\n");
    let sent = record(&reviews, &w, &worktree);
    // A's turn goes on and ends in a state of its own that keeps the credits of both files.
    write(&repo, "c.txt", "by A\n");
    let s2 = record(&reviews, &w, &worktree);
    assert_ne!(sent, s2, "the turn end is a state after the send");
    reviews
        .keep_credits(
            &w,
            &s2,
            &[credited("b.txt", &[&a]), credited("c.txt", &[&a])],
        )
        .expect("keep");
    // B's turn then changes b.txt and keeps its credit, after the range.
    write(&repo, "b.txt", "by B\n");
    let s3 = record(&reviews, &w, &worktree);
    reviews
        .keep_credits(&w, &s3, &[credited("b.txt", &[&b])])
        .expect("keep");

    // The wait of the send ended at s2: B's credit for b.txt is not A's, and B's review
    // shows nothing of what B had not touched by s2.
    assert_eq!(
        reviews.credited_change(&r.a, &s0, &s2).expect("A"),
        [
            listed("b.txt", Change::Added, &[&a], false),
            listed("c.txt", Change::Added, &[&a], false),
        ]
    );
    assert!(
        reviews
            .credited_change(&r.b, &s0, &s2)
            .expect("B")
            .is_empty()
    );
}

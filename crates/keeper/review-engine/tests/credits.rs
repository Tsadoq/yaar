//! Who is credited with each path, kept with the recorded states, and what one review shows
//! of a change between two versions, through the engine with the real Store and Recorder on a
//! repository the test makes.

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
    // A workspace with sessions has no review of no agent of its own; the Store can hold one
    // all the same, as one a session left when it took another over.
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

    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let r = reviewers(&reviews, &store);
    let modified =
        |path: &str, by: &[&str], shared: bool| listed(path, Change::Modified, by, shared);

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

    assert_eq!(
        reviews.credited_change(&r.a, &s0, &copy).expect("A"),
        added(&[])
    );
    assert_eq!(
        reviews.credited_change(&r.b, &s0, &copy).expect("B"),
        added(&[])
    );

    touch(&repo, &["b.txt"]);
    let s2 = record(&reviews, &w, &worktree);
    reviews
        .keep_credits(
            &w,
            &s2,
            &[credited("a.txt", &[&a]), credited("b.txt", &[&b])],
        )
        .expect("keep the credits");

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
    assert_eq!(change(&r.a, &s0, &s1), x(&[&a]));
    assert!(change(&r.b, &s0, &s1).is_empty());
    assert_eq!(change(&r.a, &s0, &s2), x(&[&a, &b]));
    assert_eq!(change(&r.b, &s0, &s2), x(&[&a, &b]));
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

    write(&repo, "a.txt", "by A\n");
    commit_all(&repo, "A's commit");
    let s1 = record(&reviews, &w, &worktree);
    reviews.keep_credits(&w, &s1, &[]).expect("keep none");

    write(&repo, "a.txt", "by B\n");
    let s2 = record(&reviews, &w, &worktree);
    reviews
        .keep_credits(&w, &s2, &[credited("a.txt", &[&b])])
        .expect("keep");

    // The credits B's turn keeps at s2 are not taken for s1: a.txt is no agent's there.
    assert_eq!(
        reviews.credited_change(&r.a, &s0, &s1).expect("A"),
        [listed("a.txt", Change::Added, &[], false)]
    );
    assert_eq!(
        reviews.credited_change(&r.b, &s0, &s1).expect("B"),
        [listed("a.txt", Change::Added, &[], false)]
    );
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

    write(&repo, "a.txt", "by A\n");
    commit_all(&repo, "A's commit");
    let s1 = record(&reviews, &w, &worktree);
    reviews.keep_credits(&w, &s1, &[]).expect("keep none");
    write(&repo, "b.txt", "by A\n");
    let sent = record(&reviews, &w, &worktree);
    // The turn goes on with a write of its own: the Recorder gives back the newest state of an
    // unchanged tree, so the end would otherwise be the state of the send again.
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

    write(&repo, "b.txt", "by A\n");
    let sent = record(&reviews, &w, &worktree);
    // A's turn then writes c.txt and commits everything: it ends on a clean worktree, so it keeps
    // no credit. The write makes its end a state of its own; a commit alone leaves the tree as it
    // was.
    write(&repo, "c.txt", "by A\n");
    commit_all(&repo, "A's commit");
    let s2 = record(&reviews, &w, &worktree);
    assert_ne!(sent, s2, "the clean turn end is a state after the send");
    reviews.keep_credits(&w, &s2, &[]).expect("keep none");
    write(&repo, "b.txt", "by B\n");
    let s3 = record(&reviews, &w, &worktree);
    reviews
        .keep_credits(&w, &s3, &[credited("b.txt", &[&b])])
        .expect("keep");

    // The send waits only for the end of the turn it was in, which kept none.
    let added = [listed("b.txt", Change::Added, &[], false)];
    assert_eq!(reviews.credited_change(&r.a, &s0, &sent).expect("A"), added);
    assert_eq!(reviews.credited_change(&r.b, &s0, &sent).expect("B"), added);
}

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

    write(&repo, "b.txt", "by A\n");
    let sent = record(&reviews, &w, &worktree);
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
    write(&repo, "b.txt", "by B\n");
    let s3 = record(&reviews, &w, &worktree);
    reviews
        .keep_credits(&w, &s3, &[credited("b.txt", &[&b])])
        .expect("keep");

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

#[test]
fn a_commit_as_the_newer_version_counts_the_credits_of_every_state_after_the_older_one() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, &["x.txt"]);
    let worktree = open(&repo);
    let w = workspace();
    let (a, b) = (session_id(1), session_id(2));
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let r = reviewers(&reviews, &store);
    let s0 = record(&reviews, &w, &worktree);
    write(&repo, "x.txt", "by A\n");
    let s1 = record(&reviews, &w, &worktree);
    reviews
        .keep_credits(&w, &s1, &[credited("x.txt", &[&a])])
        .expect("keep");
    write(&repo, "y.txt", "by B\n");
    let s2 = record(&reviews, &w, &worktree);
    reviews
        .keep_credits(&w, &s2, &[credited("y.txt", &[&b])])
        .expect("keep");
    commit_all(&repo, "second");
    let second = support::git(&repo, &["rev-parse", "HEAD"]);
    assert!(![&s0, &s1, &s2].contains(&&second));
    reviews
        .states_of(&w)
        .expect("the state store")
        .keep_commit(&worktree, &yaar_recorder::CommitId::new(&second))
        .expect("keep the commit");

    // The commit counts as the newest state, though no state of the list is the commit.
    assert_eq!(
        reviews
            .credited_change(&r.nobody, &s0, &second)
            .expect("no agent"),
        [
            listed("x.txt", Change::Modified, &[&a], false),
            listed("y.txt", Change::Added, &[&b], false),
        ]
    );
    assert_eq!(
        reviews.credited_change(&r.a, &s0, &second).expect("A"),
        [listed("x.txt", Change::Modified, &[&a], false)]
    );
    assert_eq!(
        reviews.credited_change(&r.b, &s0, &second).expect("B"),
        [listed("y.txt", Change::Added, &[&b], false)]
    );
}

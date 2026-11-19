//! A turn that ends, through the engine with the real Store and the real Recorder beneath,
//! on a repository the test makes.

mod support;

use std::path::PathBuf;

use support::{
    Scratch, commit_all, engine, files_of, git, init, open, session, store, workspace, write,
};
use yaar_recorder::{Change, ChangedPath, CommitId};
use yaar_review_engine::{EngineError, SessionId};

fn added(path: &str) -> ChangedPath {
    ChangedPath {
        path: PathBuf::from(path),
        change: Change::Added,
    }
}

fn modified(path: &str) -> ChangedPath {
    ChangedPath {
        path: PathBuf::from(path),
        change: Change::Modified,
    }
}

// REV-19
#[test]
fn at_a_turn_end_what_changed_is_the_diff_from_the_state_last_marked_reviewed_to_the_new_state() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "first");
    write(&repo, "a.txt", "one, then a change\n");
    let worktree = open(&repo);
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let w = workspace();
    let started = session(&store, 1);
    let review = reviews.session_started(&started).expect("start");

    // The reviewer marks the change reviewed; then the agent edits a.txt and adds b.txt.
    let s1 = reviews.mark_reviewed(&worktree, &review).expect("mark");
    write(&repo, "a.txt", "one, then another change\n");
    write(&repo, "b.txt", "new\n");
    let head = git(&repo, &["rev-parse", "HEAD"]);

    let ended = reviews
        .turn_ended(&worktree, &started.id, None)
        .expect("the turn ended");

    // "At a turn end" a state is recorded: the newest, holding the files as they are.
    let states = reviews.states_of(&w).expect("the state store");
    let listed = states.states().expect("list the states");
    assert_eq!(listed.len(), 2);
    assert_eq!(listed.last().expect("a state").id, ended.state);
    assert_ne!(ended.state, s1);
    let on_disk = files_of(&repo);
    assert_eq!(
        states.paths(ended.state.as_str()).expect("paths"),
        on_disk.keys().cloned().collect::<Vec<PathBuf>>()
    );
    for (path, content) in &on_disk {
        assert_eq!(
            states.content(ended.state.as_str(), path).expect("read"),
            Some(content.clone())
        );
    }
    // Without a commit of the user's.
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head);

    // What changed since the last review is the diff from the state marked reviewed to the
    // new state, and it is exactly the two paths.
    assert_eq!(ended.since.as_deref(), Some(s1.as_str()));
    let diff = states
        .changed_paths(s1.as_str(), ended.state.as_str())
        .expect("the diff of the two states");
    assert_eq!(ended.changed.as_ref(), Some(&diff));
    assert_eq!(diff, vec![modified("a.txt"), added("b.txt")]);
}

#[test]
fn before_any_mark_what_changed_is_counted_from_the_start_given_and_from_nothing_without() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    let start = CommitId::new(commit_all(&repo, "first"));
    write(&repo, "a.txt", "changed\n");
    write(&repo, "b.txt", "new\n");
    let worktree = open(&repo);
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let started = session(&store, 1);
    reviews.session_started(&started).expect("start");

    let without = reviews
        .turn_ended(&worktree, &started.id, None)
        .expect("a turn ended");
    assert_eq!(without.since, None);
    assert_eq!(without.changed, None);

    let counted = reviews
        .turn_ended(&worktree, &started.id, Some(&start))
        .expect("a turn ended");
    assert_eq!(counted.since, None);
    assert_eq!(counted.state, without.state);
    assert_eq!(
        counted.changed,
        Some(vec![modified("a.txt"), added("b.txt")])
    );
}

#[test]
fn a_mark_wins_over_the_start_and_a_turn_that_changed_nothing_has_an_empty_change() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    let start = CommitId::new(commit_all(&repo, "first"));
    write(&repo, "a.txt", "changed\n");
    let worktree = open(&repo);
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let started = session(&store, 1);
    let review = reviews.session_started(&started).expect("start");
    let marked = reviews.mark_reviewed(&worktree, &review).expect("mark");

    let ended = reviews
        .turn_ended(&worktree, &started.id, Some(&start))
        .expect("a turn ended");

    // The start counts only before the first mark, though a.txt differs from it.
    assert_eq!(ended.since.as_deref(), Some(marked.as_str()));
    assert_eq!(ended.state, marked);
    assert_eq!(ended.changed, Some(vec![]));
}

#[test]
fn a_commit_made_between_the_mark_and_the_turn_end_changes_nothing_of_what_changed() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "first");
    let worktree = open(&repo);
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let started = session(&store, 1);
    let review = reviews.session_started(&started).expect("start");
    let marked = reviews.mark_reviewed(&worktree, &review).expect("mark");
    write(&repo, "a.txt", "two\n");
    worktree.commit_all("Commit the edit").expect("commit");

    let ended = reviews
        .turn_ended(&worktree, &started.id, None)
        .expect("a turn ended");

    assert_eq!(ended.since.as_deref(), Some(marked.as_str()));
    assert_eq!(ended.changed, Some(vec![modified("a.txt")]));
}

#[test]
fn each_session_counts_from_the_state_its_own_review_was_marked_at() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "first");
    let worktree = open(&repo);
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let (a, b) = (session(&store, 1), session(&store, 2));
    let review_a = reviews.session_started(&a).expect("start a");
    let review_b = reviews.session_started(&b).expect("start b");
    let marked_a = reviews.mark_reviewed(&worktree, &review_a).expect("mark");
    write(&repo, "a.txt", "two\n");
    let marked_b = reviews.mark_reviewed(&worktree, &review_b).expect("mark");
    write(&repo, "c.txt", "three\n");

    let of_a = reviews
        .turn_ended(&worktree, &a.id, None)
        .expect("a's turn");
    let of_b = reviews
        .turn_ended(&worktree, &b.id, None)
        .expect("b's turn");

    assert_eq!(of_a.since.as_deref(), Some(marked_a.as_str()));
    assert_eq!(of_a.changed, Some(vec![modified("a.txt"), added("c.txt")]));
    assert_eq!(of_b.since.as_deref(), Some(marked_b.as_str()));
    assert_eq!(of_b.changed, Some(vec![added("c.txt")]));
    assert_eq!(of_a.state, of_b.state);
}

#[test]
fn a_session_with_no_review_is_refused_and_records_nothing() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "first");
    let worktree = open(&repo);
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    // Kept in the Store, but never told to the engine as started.
    let unheard = session(&store, 1);
    let unknown = SessionId::new(support::id(99));

    for who in [&unheard.id, &unknown] {
        let refused = reviews.turn_ended(&worktree, who, None);
        assert!(
            matches!(refused, Err(EngineError::NoReviewOfSession(_))),
            "{refused:?}"
        );
    }
    assert!(
        reviews
            .states_of(&workspace())
            .expect("the state store")
            .states()
            .expect("list")
            .is_empty()
    );
}

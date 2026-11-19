//! The engine told by a watch of the worktree's files: a new thread on files nobody
//! changed records no state, and every state the engine takes is told to the watch.
//! Through the engine with the real Store and the real Recorder beneath.

mod support;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use support::{
    Scratch, commit_all, engine, git, init, new_thread, open, session, store, workspace, write,
};
use yaar_recorder::StateId;
use yaar_review_engine::{StateWatch, WorkspaceId};

/// What the Keeper's watcher of a worktree is to the engine: it says whether files changed
/// and remembers each state it was told of.
#[derive(Debug, Default)]
struct Watch {
    changed: AtomicBool,
    told: Mutex<Vec<(WorkspaceId, StateId)>>,
}

impl Watch {
    fn told(&self) -> Vec<(WorkspaceId, StateId)> {
        self.told.lock().expect("the list").clone()
    }
}

impl StateWatch for Watch {
    fn changed_since_state(&self, _workspace: &WorkspaceId) -> bool {
        self.changed.load(Ordering::SeqCst)
    }

    fn state_taken(&self, workspace: &WorkspaceId, state: &StateId) {
        self.told
            .lock()
            .expect("the list")
            .push((workspace.clone(), state.clone()));
    }
}

#[test]
fn every_state_the_engine_records_is_told_to_the_watch() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "first");
    let worktree = open(&repo);
    let store = store(&scratch);
    let watch = Arc::new(Watch::default());
    watch.changed.store(true, Ordering::SeqCst);
    let reviews = engine(&scratch, store.clone()).watching(watch.clone());
    let w = workspace();
    let started = session(&store, 1);
    let review = reviews.session_started(&started).expect("start");
    assert!(
        watch.told().is_empty(),
        "starting a session records nothing"
    );

    let marked = reviews.mark_reviewed(&worktree, &review).expect("mark");
    write(&repo, "a.txt", "two\n");
    let ended = reviews
        .turn_ended(&worktree, &started.id, None)
        .expect("a turn ended");
    write(&repo, "a.txt", "three\n");
    reviews
        .changed_since_review(&worktree, &review)
        .expect("ask");
    write(&repo, "a.txt", "four\n");
    reviews
        .save_draft(
            Some(&worktree),
            &review,
            &new_thread(1, "a.txt", 1, "four", "x"),
        )
        .expect("a thread on the files");
    write(&repo, "a.txt", "five\n");
    reviews.copy_as_text(&worktree, &review).expect("copy");

    let told = watch.told();
    assert_eq!(told.len(), 5, "{told:?}");
    assert!(told.iter().all(|(workspace, _)| workspace == &w));
    // In the order they were taken, the ones the calls returned among them.
    assert_eq!(told[0].1, marked);
    assert_eq!(told[1].1, ended.state);
    // Each is a state the store holds, and they are the store's, oldest first.
    let kept: Vec<StateId> = reviews
        .states_of(&w)
        .expect("the state store")
        .states()
        .expect("list")
        .into_iter()
        .map(|state| state.id)
        .collect();
    let told: Vec<StateId> = told.into_iter().map(|(_, state)| state).collect();
    assert_eq!(told, kept);
}

#[test]
fn a_new_thread_on_files_nobody_changed_sits_on_the_newest_state_and_records_none() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\ntwo\n");
    commit_all(&repo, "first");
    let worktree = open(&repo);
    let store = store(&scratch);
    let watch = Arc::new(Watch::default());
    let reviews = engine(&scratch, store.clone()).watching(watch.clone());
    let w = workspace();
    let review = reviews
        .of_workspace(&w, Some(&worktree))
        .expect("read")
        .reviews[0]
        .id()
        .clone();
    let newest = reviews.mark_reviewed(&worktree, &review).expect("mark");
    let told = watch.told().len();

    // The files stayed as they were: the watch says so, and so does the repository.
    reviews
        .save_draft(
            Some(&worktree),
            &review,
            &new_thread(1, "a.txt", 2, "two", "x"),
        )
        .expect("a thread");

    let thread = &reviews.review(&review).expect("read").threads()[0].clone();
    assert_eq!(thread.anchor.tree, newest.as_str());
    let states = reviews.states_of(&w).expect("the state store");
    assert_eq!(states.states().expect("list").len(), 1);
    assert_eq!(watch.told().len(), told, "no state was taken");

    // The engine believes the watch: with a file edited behind its back, a thread still
    // sits on the newest state and the store records nothing, which is how this shows
    // that it did not look at the files at all.
    write(&repo, "a.txt", "one\nbehind the watch's back\n");
    reviews
        .save_draft(
            Some(&worktree),
            &review,
            &new_thread(2, "a.txt", 1, "one", "y"),
        )
        .expect("a second thread");
    let second = reviews.review(&review).expect("read").threads()[1].clone();
    assert_eq!(second.anchor.tree, newest.as_str());
    assert_eq!(states.states().expect("list").len(), 1);
}

#[test]
fn a_new_thread_records_a_state_when_the_watch_saw_files_change() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\ntwo\n");
    commit_all(&repo, "first");
    let worktree = open(&repo);
    let store = store(&scratch);
    let watch = Arc::new(Watch::default());
    let reviews = engine(&scratch, store.clone()).watching(watch.clone());
    let w = workspace();
    let review = reviews
        .of_workspace(&w, Some(&worktree))
        .expect("read")
        .reviews[0]
        .id()
        .clone();
    let before = reviews.mark_reviewed(&worktree, &review).expect("mark");
    write(&repo, "a.txt", "one\ntwo changed\n");
    watch.changed.store(true, Ordering::SeqCst);

    reviews
        .save_draft(
            Some(&worktree),
            &review,
            &new_thread(1, "a.txt", 2, "two changed", "x"),
        )
        .expect("a thread");

    let thread = reviews.review(&review).expect("read").threads()[0].clone();
    assert_ne!(thread.anchor.tree, before.as_str());
    let states = reviews.states_of(&w).expect("the state store");
    let listed = states.states().expect("list");
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[1].id.as_str(), thread.anchor.tree);
    assert_eq!(
        states
            .content(&thread.anchor.tree, std::path::Path::new("a.txt"))
            .expect("read"),
        Some(b"one\ntwo changed\n".to_vec())
    );
    assert_eq!(
        watch.told().last().map(|(_, state)| state),
        Some(&listed[1].id)
    );
}

#[test]
fn a_new_thread_records_the_first_state_though_the_watch_saw_no_change() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "first");
    let worktree = open(&repo);
    let store = store(&scratch);
    let watch = Arc::new(Watch::default());
    let reviews = engine(&scratch, store.clone()).watching(watch.clone());
    let w = workspace();
    let review = reviews
        .of_workspace(&w, Some(&worktree))
        .expect("read")
        .reviews[0]
        .id()
        .clone();
    let head = git(&repo, &["rev-parse", "HEAD"]);

    reviews
        .save_draft(
            Some(&worktree),
            &review,
            &new_thread(1, "a.txt", 1, "one", "x"),
        )
        .expect("a thread");

    // The store held no state, so there was none to sit on.
    let listed = reviews
        .states_of(&w)
        .expect("the state store")
        .states()
        .expect("list");
    assert_eq!(listed.len(), 1);
    let thread = reviews.review(&review).expect("read").threads()[0].clone();
    assert_eq!(thread.anchor.tree, listed[0].id.as_str());
    assert_eq!(watch.told().len(), 1);
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head);
}

#[test]
fn an_engine_with_no_watch_records_a_state_for_each_new_thread_on_the_files_on_disk() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "first");
    let worktree = open(&repo);
    let reviews = engine(&scratch, store(&scratch));
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
            &new_thread(1, "a.txt", 1, "one", "x"),
        )
        .expect("a thread");
    write(&repo, "a.txt", "two\n");
    reviews
        .save_draft(
            Some(&worktree),
            &review,
            &new_thread(2, "a.txt", 1, "two", "y"),
        )
        .expect("a thread");

    let listed = reviews
        .states_of(&w)
        .expect("the state store")
        .states()
        .expect("list");
    assert_eq!(listed.len(), 2);
}

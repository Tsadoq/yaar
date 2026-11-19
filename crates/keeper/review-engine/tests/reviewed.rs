//! Marking a change reviewed, through the engine with the real Store and the real
//! Recorder beneath, on a repository the test makes.

mod support;

use std::path::PathBuf;

use support::{Scratch, commit_all, engine, files_of, git, init, open, store, workspace, write};
use yaar_recorder::{Change, ChangedPath};

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

// REV-16
#[test]
fn marking_reviewed_records_the_state_as_reviewed_and_a_commit_made_through_the_app_changes_neither()
 {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "first");
    write(&repo, "a.txt", "one changed\n");
    let worktree = open(&repo);

    let reviews = engine(&scratch, store(&scratch));
    let w = workspace();
    let review = reviews
        .of_workspace(&w, Some(&worktree))
        .expect("read")
        .reviews[0]
        .id()
        .clone();

    // Before the first mark there is no reviewed state, and nothing to compare with.
    assert_eq!(reviews.review(&review).expect("read").reviewed(), None);
    assert_eq!(
        reviews
            .changed_since_review(&worktree, &review)
            .expect("ask"),
        None
    );

    // An earlier mark, then an edit: the mark below must be a later state than the oldest
    // one the workspace has recorded, so that what the next review starts from is the
    // reviewed state and no other.
    let earlier = reviews.mark_reviewed(&worktree, &review).expect("mark");
    write(&repo, "a.txt", "one changed once more\n");

    let head = git(&repo, &["rev-parse", "HEAD"]);
    let refs = git(
        &repo,
        &["for-each-ref", "--format=%(refname) %(objectname)"],
    );
    let objects = git(&repo, &["count-objects", "-v"]);

    // Marking records the current state as reviewed: the newest state, holding the files
    // as they are.
    let reviewed = reviews.mark_reviewed(&worktree, &review).expect("mark");
    assert_ne!(reviewed, earlier);
    assert_eq!(
        reviews.review(&review).expect("read").reviewed(),
        Some(reviewed.as_str())
    );
    let states = reviews.states_of(&w).expect("the state store");
    let listed = states.states().expect("list the states");
    assert_eq!(listed.last().expect("a state").id, reviewed);
    let on_disk = files_of(&repo);
    assert_eq!(
        states.paths(reviewed.as_str()).expect("paths"),
        on_disk.keys().cloned().collect::<Vec<PathBuf>>()
    );
    for (path, content) in &on_disk {
        assert_eq!(
            states.content(reviewed.as_str(), path).expect("read"),
            Some(content.clone())
        );
    }

    // Without making a commit: HEAD, the refs and the objects of the repository are as
    // they were.
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        git(
            &repo,
            &["for-each-ref", "--format=%(refname) %(objectname)"]
        ),
        refs
    );
    assert_eq!(git(&repo, &["count-objects", "-v"]), objects);

    // The next review starts from it: nothing has changed since, and then exactly what
    // was edited and added.
    assert_eq!(
        reviews
            .changed_since_review(&worktree, &review)
            .expect("ask"),
        Some(vec![])
    );
    write(&repo, "a.txt", "one changed twice\n");
    write(&repo, "b.txt", "new\n");
    let since = vec![modified("a.txt"), added("b.txt")];
    assert_eq!(
        reviews
            .changed_since_review(&worktree, &review)
            .expect("ask"),
        Some(since.clone())
    );

    // A commit made through the app changes neither: the reviewed state is the same, and
    // so is what changed since it.
    let commit = worktree.commit_all("Commit what is there").expect("commit");
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), commit.as_str());
    assert_ne!(commit.as_str(), head);
    assert_eq!(
        reviews.review(&review).expect("read").reviewed(),
        Some(reviewed.as_str())
    );
    assert_eq!(
        reviews
            .changed_since_review(&worktree, &review)
            .expect("ask"),
        Some(since)
    );
}

#[test]
fn marking_reviewed_twice_on_unchanged_files_is_the_same_state() {
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
    let first = reviews.mark_reviewed(&worktree, &review).expect("mark");
    let second = reviews
        .mark_reviewed(&worktree, &review)
        .expect("mark again");
    assert_eq!(first, second);
    assert_eq!(
        reviews
            .states_of(&w)
            .expect("the state store")
            .states()
            .expect("list")
            .len(),
        1
    );

    write(&repo, "a.txt", "two\n");
    let third = reviews
        .mark_reviewed(&worktree, &review)
        .expect("mark a third time");
    assert_ne!(third, first);
    assert_eq!(
        reviews.review(&review).expect("read").reviewed(),
        Some(third.as_str())
    );
}

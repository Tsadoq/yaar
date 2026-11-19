//! The files marked viewed, through the engine with the real Store and the real Recorder
//! beneath, on a repository the test makes.

mod support;

use std::path::{Path, PathBuf};

use support::{Scratch, commit_all, engine, init, open, store, workspace, write};
use yaar_review_engine::EngineError;

fn paths(paths: &[&str]) -> Vec<PathBuf> {
    paths.iter().map(PathBuf::from).collect()
}

#[test]
fn a_viewed_file_loses_the_mark_when_its_content_changes_and_keeps_it_when_another_file_changes() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.rs", "one\n");
    write(&repo, "b.rs", "two\n");
    write(&repo, "src/c.rs", "three\n");
    commit_all(&repo, "first");
    let worktree = open(&repo);
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let w = workspace();

    for file in ["a.rs", "b.rs", "src/c.rs"] {
        reviews
            .set_viewed(&w, &worktree, Path::new(file), true)
            .expect("mark");
    }
    let view = reviews.of_workspace(&w, Some(&worktree)).expect("read");
    assert_eq!(view.viewed, paths(&["a.rs", "b.rs", "src/c.rs"]));

    // One file changes: only its mark goes, and the Store no longer holds it.
    write(&repo, "a.rs", "one, changed\n");
    let view = reviews.of_workspace(&w, Some(&worktree)).expect("read");
    assert_eq!(view.viewed, paths(&["b.rs", "src/c.rs"]));
    let kept: Vec<PathBuf> = store
        .viewed(&w)
        .expect("kept")
        .into_iter()
        .map(|record| record.path)
        .collect();
    assert_eq!(kept, paths(&["b.rs", "src/c.rs"]));

    // A file that is gone loses its mark too; one marked again is viewed for its new content.
    std::fs::remove_file(repo.join("b.rs")).expect("remove");
    reviews
        .set_viewed(&w, &worktree, Path::new("a.rs"), true)
        .expect("mark again");
    let view = reviews.of_workspace(&w, Some(&worktree)).expect("read");
    assert_eq!(view.viewed, paths(&["a.rs", "src/c.rs"]));

    // The mark is taken away, and taking away a mark that is not there is no error.
    reviews
        .set_viewed(&w, &worktree, Path::new("a.rs"), false)
        .expect("unmark");
    reviews
        .set_viewed(&w, &worktree, Path::new("a.rs"), false)
        .expect("unmark again");
    let view = reviews.of_workspace(&w, Some(&worktree)).expect("read");
    assert_eq!(view.viewed, paths(&["src/c.rs"]));
}

#[test]
fn a_workspace_with_no_worktree_shows_no_file_viewed_and_loses_no_mark() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.rs", "one\n");
    commit_all(&repo, "first");
    let worktree = open(&repo);
    let reviews = engine(&scratch, store(&scratch));
    let w = workspace();
    reviews
        .set_viewed(&w, &worktree, Path::new("a.rs"), true)
        .expect("mark");

    let gone = reviews.of_workspace(&w, None).expect("read");
    assert!(gone.viewed.is_empty());

    let back = reviews.of_workspace(&w, Some(&worktree)).expect("read");
    assert_eq!(back.viewed, paths(&["a.rs"]));
}

#[test]
fn a_path_that_leads_out_of_the_worktree_or_names_no_file_cannot_be_marked() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.rs", "one\n");
    write(scratch.path(), "outside.rs", "secret\n");
    commit_all(&repo, "first");
    let worktree = open(&repo);
    let store = store(&scratch);
    let reviews = engine(&scratch, store.clone());
    let w = workspace();

    for bad in ["", "../outside.rs", "/etc/passwd", "a/../a.rs", "./a.rs"] {
        let refused = reviews.set_viewed(&w, &worktree, Path::new(bad), true);
        assert!(
            matches!(refused, Err(EngineError::OutsideWorktree(_))),
            "{bad:?}: {refused:?}"
        );
    }
    assert!(store.viewed(&w).expect("kept").is_empty());
}

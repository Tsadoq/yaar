//! Component tests: a worktree that git moves or removes, and the reviews the Store keeps
//! for it, through the calls the Keeper makes, in real repositories the tests make and
//! with a real database file.

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use support::{Scratch, commit_all, git, git_that_keeps_a_list, init, write};
use yaar_recorder::Git;
use yaar_store::{
    Anchor, Author, Comment, CommentId, FILE_NAME, ReviewId, ReviewRecord, Side, Store, Thread,
    ThreadId,
};
use yaar_workspaces::{
    Head, Refusal, Repository, Workspace, WorkspaceError, WorkspaceId, Workspaces,
};

/// An id of 32 hexadecimal digits, from a number.
fn id(n: u128) -> String {
    format!("{n:032x}")
}

/// The database file of the test's Keeper.
fn file(scratch: &Scratch) -> PathBuf {
    scratch.join("app-data").join(FILE_NAME)
}

/// A Keeper's Workspaces over the database file of the scratch folder, with the Store it
/// reads, which the test uses as the Review engine would. Making another is a Keeper
/// that restarts: nothing but the file and the repositories is carried over.
fn keeper(scratch: &Scratch) -> (Workspaces, Store) {
    keeper_through(scratch, Git::find().expect("git on the search path"))
}

fn keeper_through(scratch: &Scratch, git: Git) -> (Workspaces, Store) {
    let store = Store::open(&file(scratch)).expect("open the database");
    (Workspaces::new(git, store.clone()), store)
}

/// A review of `workspace` that holds one thread, whose one comment is a draft, put
/// through the Store's calls as the engine puts one.
fn review_with_a_thread(store: &Store, workspace: &WorkspaceId, n: u128) -> ReviewRecord {
    let thread = Thread {
        id: ThreadId::new(id(n + 1)),
        anchor: Anchor {
            tree: "a".repeat(40),
            path: PathBuf::from("src/a.txt"),
            side: Side::New,
            first: 2,
            last: 3,
            text: "two\nthree".to_owned(),
        },
        resolved: false,
        addressed: false,
        comments: vec![Comment {
            id: CommentId::new(id(n + 2)),
            author: Author::Reviewer,
            text: "Why is this here?".to_owned(),
            round: None,
        }],
    };
    let empty = ReviewRecord {
        id: ReviewId::new(id(n)),
        workspace: workspace.clone(),
        agent: None,
        reviewed: None,
        threads: vec![],
        rounds: vec![],
    };
    store.put_review(&empty).expect("put the review");
    store
        .put_thread(&empty.id, &thread)
        .expect("put the thread");
    ReviewRecord {
        threads: vec![thread],
        ..empty
    }
}

/// A review of `workspace` with no thread: the review of no agent that every workspace
/// has from its first reading.
fn review_with_no_thread(store: &Store, workspace: &WorkspaceId, n: u128) {
    store
        .put_review(&ReviewRecord {
            id: ReviewId::new(id(n)),
            workspace: workspace.clone(),
            agent: None,
            reviewed: None,
            threads: vec![],
            rounds: vec![],
        })
        .expect("put the review");
}

/// A repository in `repo` with one commit on `main` and its linked worktrees, one for
/// each name, each on a branch of its name in the folder of its name.
fn a_repository_with_worktrees(scratch: &Scratch, names: &[&str]) -> PathBuf {
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "src/a.txt", "one\ntwo\nthree\n");
    commit_all(&repo, "first");
    for name in names {
        git(
            &repo,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                name,
                &format!("../{name}"),
            ],
        );
    }
    repo
}

fn the_repository(workspaces: &Workspaces) -> Repository {
    let mut all = workspaces.repositories().expect("repositories");
    assert_eq!(all.len(), 1, "{all:?}");
    all.remove(0)
}

fn listed_ids(repository: &Repository) -> Vec<&WorkspaceId> {
    repository.workspaces.iter().map(|w| &w.id).collect()
}

fn refused(result: Result<impl std::fmt::Debug, WorkspaceError>) -> Refusal {
    match result {
        Err(WorkspaceError::Refused(refusal)) => refusal,
        other => panic!("not refused: {other:?}"),
    }
}

fn on_branch(workspace: &Workspace, name: &str) {
    assert!(
        matches!(&workspace.head, Some(Head::Branch { name: n, .. }) if n == name),
        "{workspace:?}"
    );
}

fn in_its_folder(workspaces: &Workspaces, id: &WorkspaceId, folder: &Path) {
    let worktree = workspaces
        .worktree(id)
        .expect("worktree")
        .expect("the worktree is found at the new path");
    assert_eq!(worktree.root(), folder);
}

// STORE-05.
#[test]
fn after_git_moves_a_worktree_opening_the_new_path_finds_its_review() {
    let scratch = Scratch::new();
    let repo = a_repository_with_worktrees(&scratch, &["wt"]);
    let (workspaces, store) = keeper(&scratch);
    let opened = workspaces
        .open_folder(&scratch.join("wt"))
        .expect("open the worktree");
    let review = review_with_a_thread(&store, &opened.workspace, 0x10);

    git(&repo, &["worktree", "move", "../wt", "../wt-moved"]);
    let moved = scratch.join("wt-moved");

    // The same workspace is found at the new path, listed there with its head, and its
    // review, the thread in it, is as it was.
    let again = workspaces.open_folder(&moved).expect("open the new path");
    assert_eq!(again, opened);
    // The open alone is enough to use the workspace at its new path, before any listing.
    in_its_folder(&workspaces, &opened.workspace, &moved);
    let repository = the_repository(&workspaces);
    let at_the_new_path = repository
        .workspaces
        .iter()
        .find(|w| w.id == opened.workspace)
        .expect("the workspace is listed");
    assert_eq!(at_the_new_path.path, moved);
    on_branch(at_the_new_path, "wt");
    assert_eq!(
        repository.workspaces.len(),
        2,
        "the old path is listed no more: {repository:?}"
    );
    assert_eq!(
        store.reviews_of(&opened.workspace).expect("reviews"),
        std::slice::from_ref(&review)
    );

    // After a restart: a second Workspaces on a second Store of the same file.
    drop((workspaces, store));
    let (workspaces, store) = keeper(&scratch);
    assert_eq!(workspaces.open_folder(&moved).expect("open"), opened);
    in_its_folder(&workspaces, &opened.workspace, &moved);
    let listed = the_repository(&workspaces);
    assert!(
        listed
            .workspaces
            .iter()
            .any(|w| w.id == opened.workspace && w.path == moved && w.head.is_some()),
        "{listed:?}"
    );
    assert_eq!(
        store.reviews_of(&opened.workspace).expect("reviews"),
        [review]
    );
}

// STORE-06.
#[test]
fn after_git_removes_a_worktree_its_review_is_still_listed_marked_as_worktree_gone() {
    let scratch = Scratch::new();
    let repo = a_repository_with_worktrees(&scratch, &["kept", "bare"]);
    let (workspaces, store) = keeper(&scratch);
    let main = workspaces
        .open_folder(&repo)
        .expect("open the main worktree");
    let kept = workspaces
        .open_folder(&scratch.join("kept"))
        .expect("open kept");
    let bare = workspaces
        .open_folder(&scratch.join("bare"))
        .expect("open bare");
    // The main worktree and `bare` have a review each, with no thread in it.
    review_with_no_thread(&store, &main.workspace, 0x20);
    review_with_no_thread(&store, &bare.workspace, 0x30);
    let review = review_with_a_thread(&store, &kept.workspace, 0x40);
    let before = the_repository(&workspaces);
    let mut all_three = listed_ids(&before);
    all_three.sort();
    let mut opened_three = [&main.workspace, &kept.workspace, &bare.workspace];
    opened_three.sort();
    assert_eq!(all_three, opened_three);

    git(&repo, &["worktree", "remove", "../kept"]);
    git(&repo, &["worktree", "remove", "../bare"]);

    let check = |workspaces: &Workspaces, store: &Store| {
        let repository = the_repository(workspaces);
        // Still listed at the path it had; the one with no thread is not; the main
        // worktree is listed as it was.
        assert_eq!(
            repository.workspaces,
            [
                before.workspaces[0].clone(),
                Workspace {
                    id: kept.workspace.clone(),
                    path: scratch.join("kept"),
                    head: None,
                }
            ]
        );
        assert!(repository.workspaces[0].head.is_some());
        assert_eq!(
            store.reviews_of(&kept.workspace).expect("reviews"),
            std::slice::from_ref(&review)
        );
        // Nothing that needs its files is possible, as for an id nobody knows.
        assert_eq!(
            refused(workspaces.changes(&kept.workspace)),
            Refusal::UnknownWorkspace(kept.workspace.clone())
        );
        // The review of the one with no thread is kept all the same.
        assert_eq!(store.reviews_of(&bare.workspace).expect("reviews").len(), 1);
    };
    check(&workspaces, &store);

    // After a restart: a second Workspaces on a second Store of the same file.
    drop((workspaces, store));
    let (workspaces, store) = keeper(&scratch);
    check(&workspaces, &store);
}

#[test]
fn a_gone_worktree_is_listed_after_the_listed_ones_in_order_and_leaves_with_its_last_thread() {
    let scratch = Scratch::new();
    let repo = a_repository_with_worktrees(&scratch, &["one", "two", "three"]);
    let (workspaces, store) = keeper(&scratch);
    let one = workspaces.open_folder(&scratch.join("one")).expect("one");
    let two = workspaces.open_folder(&scratch.join("two")).expect("two");
    let three = workspaces
        .open_folder(&scratch.join("three"))
        .expect("three");
    let main = workspaces.open_folder(&repo).expect("main");
    review_with_a_thread(&store, &one.workspace, 0x10);
    let in_two = review_with_a_thread(&store, &two.workspace, 0x20);

    // `one` is removed by git, `two` is deleted from the drive: both are gone, whichever
    // way, and both come after the worktrees that are listed.
    git(&repo, &["worktree", "remove", "../one"]);
    std::fs::remove_dir_all(scratch.join("two")).expect("delete the folder");
    let repository = the_repository(&workspaces);
    let gone: Vec<&Workspace> = repository
        .workspaces
        .iter()
        .filter(|w| w.head.is_none())
        .collect();
    assert_eq!(gone.len(), 2, "{repository:?}");
    assert_eq!(repository.workspaces.len(), 4, "{repository:?}");
    assert!(repository.workspaces[..2].iter().all(|w| w.head.is_some()));
    let mut listed = listed_ids(&repository)[..2].to_vec();
    listed.sort();
    let mut alive = [&main.workspace, &three.workspace];
    alive.sort();
    assert_eq!(listed, alive);
    let paths: Vec<&Path> = gone.iter().map(|w| w.path.as_path()).collect();
    assert_eq!(paths.len(), 2);
    assert!(paths.contains(&scratch.join("one").as_path()));
    assert!(paths.contains(&scratch.join("two").as_path()));

    // The last draft of `two` is deleted, which takes its thread with it: `two` is not
    // listed any more, and the Store still holds its review.
    store
        .delete_draft(&in_two.threads[0].comments[0].id)
        .expect("delete the draft");
    let repository = the_repository(&workspaces);
    assert!(
        repository.workspaces.iter().all(|w| w.id != two.workspace),
        "{repository:?}"
    );
    assert!(repository.workspaces.iter().any(|w| w.id == one.workspace));
    assert_eq!(store.reviews_of(&two.workspace).expect("reviews").len(), 1);
}

#[test]
fn a_repository_whose_folder_is_gone_lists_its_gone_workspaces_that_hold_a_thread_and_nothing_else()
{
    let scratch = Scratch::new();
    let repo = a_repository_with_worktrees(&scratch, &[]);
    let (workspaces, store) = keeper(&scratch);
    let opened = workspaces.open_folder(&repo).expect("open");
    review_with_a_thread(&store, &opened.workspace, 0x10);
    std::fs::remove_dir_all(&repo).expect("delete the repository");

    let repository = the_repository(&workspaces);
    assert_eq!(
        repository.workspaces,
        [Workspace {
            id: opened.workspace.clone(),
            path: repo.clone(),
            head: None,
        }]
    );

    // With no thread left it lists nothing, as before.
    let other = scratch.join("other");
    init(&other);
    write(&other, "a.txt", "a\n");
    commit_all(&other, "first");
    let in_other = workspaces.open_folder(&other).expect("open");
    review_with_no_thread(&store, &in_other.workspace, 0x50);
    std::fs::remove_dir_all(&other).expect("delete the repository");
    let all = workspaces.repositories().expect("repositories");
    assert_eq!(all.len(), 2);
    assert_eq!(all[1].workspaces, []);
}

#[test]
fn a_gone_worktree_that_comes_back_at_its_path_is_listed_with_its_head_again() {
    let scratch = Scratch::new();
    a_repository_with_worktrees(&scratch, &["wt"]);
    let (workspaces, store) = keeper(&scratch);
    let opened = workspaces.open_folder(&scratch.join("wt")).expect("open");
    review_with_a_thread(&store, &opened.workspace, 0x10);

    // Moved by hand, so that git still lists the old folder, and then put back.
    let away = scratch.join("away");
    std::fs::rename(scratch.join("wt"), &away).expect("move the worktree");
    let repository = the_repository(&workspaces);
    let gone = repository
        .workspaces
        .iter()
        .find(|w| w.id == opened.workspace)
        .expect("listed");
    assert_eq!(gone.head, None);
    assert_eq!(gone.path, scratch.join("wt"));
    std::fs::rename(&away, scratch.join("wt")).expect("put it back");
    let repository = the_repository(&workspaces);
    let back: Vec<&Workspace> = repository
        .workspaces
        .iter()
        .filter(|w| w.id == opened.workspace)
        .collect();
    assert_eq!(back.len(), 1, "{repository:?}");
    on_branch(back[0], "wt");
}

#[test]
fn the_worktree_of_a_workspace_is_the_one_kept_and_none_once_it_is_gone() {
    let scratch = Scratch::new();
    let repo = a_repository_with_worktrees(&scratch, &["wt"]);
    let (workspaces, _store) = keeper(&scratch);
    let opened = workspaces.open_folder(&scratch.join("wt")).expect("open");

    let worktree = workspaces
        .worktree(&opened.workspace)
        .expect("worktree")
        .expect("the worktree is there");
    assert_eq!(worktree.root(), scratch.join("wt"));
    let again = workspaces
        .worktree(&opened.workspace)
        .expect("worktree")
        .expect("the worktree is there");
    assert!(Arc::ptr_eq(&worktree, &again));

    git(&repo, &["worktree", "remove", "../wt"]);
    assert!(
        workspaces
            .worktree(&opened.workspace)
            .expect("no failure")
            .is_none()
    );

    let never_held = WorkspaceId::new(id(0xdead));
    assert_eq!(
        refused(workspaces.worktree(&never_held)),
        Refusal::UnknownWorkspace(never_held)
    );
}

#[test]
fn the_worktree_of_a_workspace_is_found_in_a_new_keeper_with_no_listing_of_git() {
    let scratch = Scratch::new();
    let repo = a_repository_with_worktrees(&scratch, &["wt"]);
    let (workspaces, _store) = keeper(&scratch);
    let opened = workspaces.open_folder(&scratch.join("wt")).expect("open");
    drop(workspaces);

    // A second Keeper, whose git writes down each of its starts: asking for a worktree
    // starts no `git worktree list`, which a request of reviews may do at every call.
    let (wrapper, list) = git_that_keeps_a_list(&scratch);
    let (workspaces, _store) = keeper_through(&scratch, Git::at(wrapper));
    let worktree = workspaces
        .worktree(&opened.workspace)
        .expect("worktree")
        .expect("the worktree is there");
    assert_eq!(worktree.root(), scratch.join("wt"));
    let starts = std::fs::read_to_string(&list).unwrap_or_default();
    assert!(
        !starts.contains("worktree list"),
        "git listed the worktrees: {starts}"
    );

    // Nor does it for one that is gone.
    git(&repo, &["worktree", "remove", "../wt"]);
    let _ = std::fs::remove_file(&list);
    assert!(
        workspaces
            .worktree(&opened.workspace)
            .expect("no failure")
            .is_none()
    );
    let starts = std::fs::read_to_string(&list).unwrap_or_default();
    assert!(
        !starts.contains("worktree list"),
        "git listed the worktrees: {starts}"
    );
}

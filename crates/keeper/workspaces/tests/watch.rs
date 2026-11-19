//! Component tests: the watcher of a worktree, with the real system notices, in real
//! repositories the tests make.

mod support;

use std::path::{Path, PathBuf};

use support::{Heard, Scratch, ago, commit_all, git, init, wait_until, workspaces, write};
use yaar_workspaces::{Refusal, WorkspaceError, WorkspaceId};

/// A repository with a file committed, and the workspace of it opened but not watched.
fn opened(scratch: &Scratch) -> (yaar_workspaces::Workspaces, WorkspaceId, PathBuf) {
    let root = scratch.join("repo");
    init(&root);
    write(&root, "kept.txt", "kept\n");
    commit_all(&root, "first");
    let workspaces = workspaces(scratch);
    let workspace = workspaces.open_folder(&root).expect("open").workspace;
    (workspaces, workspace, root)
}

#[test]
fn nothing_is_watched_until_a_workspace_is_asked_for_and_it_stays_so_after_unwatching() {
    let scratch = Scratch::new();
    let (workspaces, workspace, _) = opened(&scratch);
    // Opening, listing and reading the changes start no watcher.
    workspaces.repositories().expect("repositories");
    workspaces.changes(&workspace).expect("changes");
    assert!(workspaces.watched().is_empty());

    let heard = Heard::default();
    workspaces.watch(&workspace, heard.told()).expect("watch");
    assert_eq!(workspaces.watched(), [workspace.clone()]);
    workspaces.unwatch(&workspace);
    assert!(workspaces.watched().is_empty());
    // Unwatching what is not watched is nothing.
    workspaces.unwatch(&workspace);
}

#[test]
fn a_written_file_is_told_as_files_and_a_commit_as_the_head_and_the_git_folder_is_no_file() {
    let scratch = Scratch::new();
    let (workspaces, workspace, root) = opened(&scratch);
    let heard = Heard::default();
    workspaces.watch(&workspace, heard.told()).expect("watch");

    // A commit changes nothing in the files and a great deal in the git folder: objects,
    // the list of files, the ref. The notices come in order, so that once the head is
    // told, those of the objects before it have been told or dropped.
    git(
        &root,
        &["commit", "--quiet", "--allow-empty", "-m", "second"],
    );
    wait_until("the commit is told as the head", || heard.heads() > 0);
    assert_eq!(heard.files(), 0, "what git keeps of its own is no file");

    write(&root, "new.txt", "x\n");
    wait_until("the file is told", || heard.files() > 0);
}

#[test]
fn a_file_in_a_folder_made_after_the_watch_began_is_seen_by_its_path() {
    let scratch = Scratch::new();
    let (workspaces, workspace, root) = opened(&scratch);
    workspaces
        .watch(&workspace, Heard::default().told())
        .expect("watch");
    // A command runs, so that a path the watcher sees is credited to it and the credits
    // say which paths were seen. Files are written into the new folder until one of them
    // is: where the system needs a moment to watch a new folder, the first may be missed.
    workspaces.command_began(&workspace, "a", ago(1));
    std::fs::create_dir_all(root.join("made/later")).expect("make the folders");
    let mut written = 0;
    wait_until("a file of the new folder is seen", || {
        written += 1;
        write(&root, &format!("made/later/file-{written}.txt"), "x\n");
        workspaces
            .credits(&workspace)
            .expect("credits")
            .iter()
            .any(|one| one.path.starts_with("made/later") && one.by == ["a"])
    });
}

#[test]
fn a_linked_worktree_tells_its_own_head_and_the_refs_of_its_repository() {
    let scratch = Scratch::new();
    let (workspaces, _, root) = opened(&scratch);
    let linked = scratch.join("linked");
    git(
        &root,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "feature",
            linked.to_str().expect("text"),
        ],
    );
    let workspace = workspaces.open_folder(&linked).expect("open").workspace;
    let heard = Heard::default();
    workspaces.watch(&workspace, heard.told()).expect("watch");

    // A file of the main worktree is no file of this one, and a ref made from there is a
    // change of what this one sees. The file comes first: once the ref is told, the
    // notice of the file would have been too.
    write(&root, "main-only.txt", "x\n");
    git(&root, &["branch", "another"]);
    wait_until("the ref made elsewhere is told", || heard.heads() > 0);
    assert_eq!(heard.files(), 0, "a file of another worktree");

    let heads = heard.heads();
    git(
        &linked,
        &["commit", "--quiet", "--allow-empty", "-m", "in the link"],
    );
    wait_until("a commit of the linked worktree is told", || {
        heard.heads() > heads
    });
    assert_eq!(heard.files(), 0, "its git folder lies outside it: no file");

    write(&linked, "own.txt", "x\n");
    wait_until("its own file is told", || heard.files() > 0);
}

#[test]
fn watching_again_replaces_what_is_told_and_unwatching_lets_it_go() {
    let scratch = Scratch::new();
    let (workspaces, workspace, root) = opened(&scratch);
    let first = Heard::default();
    let second = Heard::default();
    workspaces.watch(&workspace, first.told()).expect("watch");
    assert_eq!(first.holders(), 2, "the watcher holds the function");
    workspaces.watch(&workspace, second.told()).expect("again");
    assert_eq!(workspaces.watched().len(), 1);
    wait_until("the replaced watcher is gone", || first.holders() == 1);

    write(&root, "x.txt", "x\n");
    wait_until("the new function is told", || second.files() > 0);

    workspaces.unwatch(&workspace);
    wait_until("the watcher is gone", || second.holders() == 1);
}

#[test]
fn dropping_workspaces_stops_every_watcher() {
    let scratch = Scratch::new();
    let (workspaces, workspace, _) = opened(&scratch);
    let heard = Heard::default();
    workspaces.watch(&workspace, heard.told()).expect("watch");
    assert_eq!(heard.holders(), 2);
    drop(workspaces);
    wait_until("the watcher is gone", || heard.holders() == 1);
}

#[test]
fn the_ledger_outlives_the_watcher() {
    let scratch = Scratch::new();
    let (workspaces, workspace, root) = opened(&scratch);
    workspaces
        .watch(&workspace, Heard::default().told())
        .expect("watch");
    write(&root, "x.txt", "x\n");
    workspaces.agent_wrote(&workspace, "a", Path::new("x.txt"));
    workspaces.unwatch(&workspace);
    let credits = workspaces.credits(&workspace).expect("credits");
    assert_eq!(credits.len(), 1);
    assert_eq!(credits[0].by, ["a"]);
}

#[test]
fn a_workspace_the_store_does_not_hold_cannot_be_watched() {
    let scratch = Scratch::new();
    let (workspaces, _, _) = opened(&scratch);
    let unknown = WorkspaceId::new("0".repeat(32));
    match workspaces.watch(&unknown, Heard::default().told()) {
        Err(WorkspaceError::Refused(Refusal::UnknownWorkspace(id))) => assert_eq!(id, unknown),
        other => panic!("not refused: {other:?}"),
    }
    assert!(workspaces.watched().is_empty());
}

#[test]
fn a_workspace_whose_worktree_is_gone_cannot_be_watched() {
    let scratch = Scratch::new();
    let (workspaces, workspace, root) = opened(&scratch);
    std::fs::remove_dir_all(&root).expect("remove the worktree");
    match workspaces.watch(&workspace, Heard::default().told()) {
        Err(WorkspaceError::Refused(Refusal::UnknownWorkspace(_))) => {}
        other => panic!("not refused: {other:?}"),
    }
}

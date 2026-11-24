mod support;

use std::path::{Path, PathBuf};

use support::{
    Scratch, commit_all, git, git_that_keeps_a_list, init, is_alive, workspaces,
    workspaces_through, write,
};
use yaar_workspaces::{
    Change, CommitId, FileSides, Opened, Refusal, WorkspaceError, WorkspaceId, Workspaces,
};

fn refused(result: Result<impl std::fmt::Debug, WorkspaceError>) -> Refusal {
    match result {
        Err(WorkspaceError::Refused(refusal)) => refusal,
        other => panic!("not refused: {other:?}"),
    }
}

fn sides(old: Option<&[u8]>, new: Option<&[u8]>) -> FileSides {
    FileSides {
        old: old.map(<[u8]>::to_vec),
        new: new.map(<[u8]>::to_vec),
    }
}

/// A repository on a branch `feat` that edited, renamed, deleted and added a file
/// since it left `main`, with one more edit and one new file not committed.
fn a_branch_with_changes(scratch: &Scratch) -> PathBuf {
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "edited.txt", "one\n");
    write(&repo, "gone.txt", "soon gone\n");
    write(
        &repo,
        "old-name.txt",
        "a file\nlong enough\nfor git\nto pair it\nwith its new name\n",
    );
    std::fs::write(repo.join("picture.bin"), [0u8, 159, 146, 150, 0, 255]).expect("write");
    commit_all(&repo, "first");
    git(&repo, &["switch", "--quiet", "-c", "feat"]);
    write(&repo, "edited.txt", "two\n");
    git(&repo, &["rm", "--quiet", "gone.txt"]);
    git(&repo, &["mv", "old-name.txt", "new-name.txt"]);
    write(&repo, "added.txt", "new\n");
    commit_all(&repo, "second");
    write(&repo, "edited.txt", "three, not committed\n");
    std::fs::write(repo.join("picture.bin"), [1u8, 0, 2]).expect("write");
    repo
}

#[test]
fn the_two_versions_of_each_changed_file_are_the_one_in_the_commit_and_the_one_on_disk() {
    let scratch = Scratch::new();
    let repo = a_branch_with_changes(&scratch);
    let workspaces = workspaces(&scratch);
    let workspace = workspaces.open_folder(&repo).expect("open").workspace;
    let changes = workspaces.changes(&workspace).expect("changes");
    let since = changes.counted_from.as_ref();
    assert_eq!(
        since.map(CommitId::as_str),
        Some(git(&repo, &["rev-parse", "main"]).as_str())
    );

    let mut read = Vec::new();
    for file in &changes.files {
        let old_path = match &file.change {
            Change::Renamed { from } => from,
            _ => &file.path,
        };
        let both = workspaces
            .file(&workspace, since, old_path, &file.path)
            .expect("file");
        read.push((file.path.to_str().expect("text"), both));
    }
    let renamed = b"a file\nlong enough\nfor git\nto pair it\nwith its new name\n";
    assert_eq!(
        read,
        vec![
            ("added.txt", sides(None, Some(b"new\n"))),
            (
                "edited.txt",
                sides(Some(b"one\n"), Some(b"three, not committed\n"))
            ),
            ("gone.txt", sides(Some(b"soon gone\n"), None)),
            ("new-name.txt", sides(Some(renamed), Some(renamed))),
            (
                "picture.bin",
                sides(Some(&[0, 159, 146, 150, 0, 255]), Some(&[1, 0, 2]))
            ),
        ]
    );

    assert_eq!(
        workspaces
            .file(
                &workspace,
                None,
                Path::new("edited.txt"),
                Path::new("edited.txt")
            )
            .expect("file"),
        sides(None, Some(b"three, not committed\n"))
    );
}

#[test]
fn a_path_that_leads_out_of_the_worktree_is_refused_on_either_side() {
    let scratch = Scratch::new();
    let repo = a_branch_with_changes(&scratch);
    write(scratch.path(), "outside.txt", "not of the worktree\n");
    let workspaces = workspaces(&scratch);
    let workspace = workspaces.open_folder(&repo).expect("open").workspace;
    let since = CommitId::new(git(&repo, &["rev-parse", "main"]));
    let inside = Path::new("edited.txt");

    let outside = scratch.join("outside.txt");
    for out in [
        Path::new("../outside.txt"),
        Path::new("sub/../../outside.txt"),
        outside.as_path(),
        Path::new(""),
    ] {
        assert_eq!(
            refused(workspaces.file(&workspace, Some(&since), out, inside)),
            Refusal::OutsideWorkspace(out.to_owned())
        );
        assert_eq!(
            refused(workspaces.file(&workspace, Some(&since), inside, out)),
            Refusal::OutsideWorkspace(out.to_owned())
        );
    }
}

#[test]
fn a_commit_git_does_not_know_is_refused_and_a_file_the_commit_lacks_is_none() {
    let scratch = Scratch::new();
    let repo = a_branch_with_changes(&scratch);
    let workspaces = workspaces(&scratch);
    let workspace = workspaces.open_folder(&repo).expect("open").workspace;
    let main = git(&repo, &["rev-parse", "main"]);
    let file = |since: &str, path: &str| {
        let since = CommitId::new(since);
        workspaces.file(&workspace, Some(&since), Path::new(path), Path::new(path))
    };

    assert_eq!(
        file(&main, "added.txt").expect("file"),
        sides(None, Some(b"new\n"))
    );
    // Only a full commit id counts: an old side is never read from another commit than the changes
    // were counted from.
    let unknown = "0123456789abcdef0123456789abcdef01234567";
    for since in [unknown, "HEAD", "main", &main[..12], "", "--all"] {
        assert_eq!(
            refused(file(since, "edited.txt")),
            Refusal::UnknownRevision(since.to_owned()),
        );
    }
    let nobody = WorkspaceId::new("f".repeat(32));
    assert_eq!(
        refused(workspaces.file(
            &nobody,
            None,
            Path::new("edited.txt"),
            Path::new("edited.txt")
        )),
        Refusal::UnknownWorkspace(nobody)
    );
}

fn content_processes(list: &Path) -> Vec<String> {
    std::fs::read_to_string(list)
        .unwrap_or_default()
        .lines()
        .filter(|line| line.contains("cat-file"))
        .map(|line| line.split(' ').next().expect("a process id").to_owned())
        .collect()
}

#[test]
fn reading_many_files_of_a_workspace_starts_one_git_process_which_ends_with_the_keeper() {
    let scratch = Scratch::new();
    let repo = a_branch_with_changes(&scratch);
    let (program, list) = git_that_keeps_a_list(&scratch);
    let workspaces = workspaces_through(&scratch, yaar_recorder::Git::at(program));
    let workspace = workspaces.open_folder(&repo).expect("open").workspace;

    for _ in 0..10 {
        let changes = workspaces.changes(&workspace).expect("changes");
        workspaces.repositories().expect("repositories");
        for path in ["edited.txt", "gone.txt", "picture.bin"] {
            let both = workspaces
                .file(
                    &workspace,
                    changes.counted_from.as_ref(),
                    Path::new(path),
                    Path::new(path),
                )
                .expect("file");
            assert!(both.old.is_some(), "{path}");
        }
    }
    let started = content_processes(&list);
    assert_eq!(started.len(), 1, "one process for 30 reads");
    assert!(is_alive(&started[0]), "it stays for the next read");

    drop(workspaces);
    assert!(!is_alive(&started[0]), "it ended with the Keeper's parts");
}

#[test]
fn several_threads_open_list_and_read_at_once_and_agree() {
    let scratch = Scratch::new();
    let repo = a_branch_with_changes(&scratch);
    git(
        &repo,
        &["worktree", "add", "--quiet", "-b", "other", "../linked"],
    );
    let linked = scratch.join("linked");
    let workspaces = workspaces(&scratch);
    let since = CommitId::new(git(&repo, &["rev-parse", "main"]));

    // Nothing was opened or listed before: every thread meets worktrees that have no id yet.
    let answers: Vec<(Opened, Opened, Vec<WorkspaceId>, FileSides)> = std::thread::scope(|s| {
        let asking: Vec<_> = (0..6)
            .map(|_| {
                let (workspaces, repo, linked, since): (&Workspaces, _, _, _) =
                    (&workspaces, &repo, &linked, &since);
                s.spawn(move || {
                    let first = workspaces.open_folder(repo).expect("open");
                    let second = workspaces.open_folder(linked).expect("open");
                    let listed = workspaces.repositories().expect("repositories");
                    let ids = listed[0].workspaces.iter().map(|w| w.id.clone()).collect();
                    workspaces.changes(&first.workspace).expect("changes");
                    let edited = Path::new("edited.txt");
                    let both = workspaces
                        .file(&first.workspace, Some(since), edited, edited)
                        .expect("file");
                    (first, second, ids, both)
                })
            })
            .collect();
        asking
            .into_iter()
            .map(|thread| thread.join().expect("a thread"))
            .collect()
    });

    let (first, second, ids, both) = &answers[0];
    assert_eq!(ids, &[first.workspace.clone(), second.workspace.clone()]);
    assert_eq!(
        both,
        &sides(Some(b"one\n"), Some(b"three, not committed\n"))
    );
    for answer in &answers {
        assert_eq!(answer, &answers[0]);
    }
    assert_eq!(workspaces.repositories().expect("repositories").len(), 1);
}

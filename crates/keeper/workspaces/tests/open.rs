//! Component tests: opening folders and listing the repositories with their worktrees,
//! through the calls the Keeper makes, in real repositories the tests make and with a
//! real database.

mod support;

use std::path::{Path, PathBuf};

use support::{Scratch, commit_all, git, init, workspaces, write};
use yaar_workspaces::{
    Head, ID_FILE, Opened, Refusal, Repository, Workspace, WorkspaceError, Workspaces,
};

fn branch(name: &str, commit: &str) -> Head {
    Head::Branch {
        name: name.to_owned(),
        commit: Some(yaar_workspaces::CommitId::new(commit)),
    }
}

fn refused(result: Result<impl std::fmt::Debug, WorkspaceError>) -> Refusal {
    match result {
        Err(WorkspaceError::Refused(refusal)) => refusal,
        other => panic!("not refused: {other:?}"),
    }
}

/// A repository in the folder `repo` with one commit on `main`, and a second worktree
/// of it in the folder `linked`, on the branch `feat`. Returns the two folders and the
/// commit.
fn a_repository_with_two_worktrees(scratch: &Scratch) -> (PathBuf, PathBuf, String) {
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "src/deep/a.txt", "a\n");
    let first = commit_all(&repo, "first");
    git(
        &repo,
        &["worktree", "add", "--quiet", "-b", "feat", "../linked"],
    );
    (repo, scratch.join("linked"), first)
}

/// The one repository that is listed.
fn the_repository(workspaces: &Workspaces) -> Repository {
    let mut all = workspaces.repositories().expect("repositories");
    assert_eq!(all.len(), 1, "{all:?}");
    all.remove(0)
}

/// The id in the id file of a git folder: 32 lowercase hexadecimal digits and a line
/// feed, checked here.
fn id_in(git_dir: &Path) -> String {
    let content = std::fs::read_to_string(git_dir.join(ID_FILE)).expect("the id file");
    let id = content.strip_suffix('\n').expect("a line feed at the end");
    assert_eq!(id.len(), 32, "{id:?}");
    assert!(
        id.bytes()
            .all(|digit| digit.is_ascii_digit() || (b'a'..=b'f').contains(&digit)),
        "{id:?}"
    );
    id.to_owned()
}

// WS-07.
#[test]
fn two_worktrees_of_one_repository_are_listed_under_it_each_named_by_its_branch() {
    let scratch = Scratch::new();
    let (repo, linked, first) = a_repository_with_two_worktrees(&scratch);
    let workspaces = workspaces(&scratch);
    assert_eq!(workspaces.repositories().expect("repositories"), vec![]);

    // One of the two is opened; the other is listed because git lists it.
    let opened = workspaces.open_folder(&repo).expect("open the folder");
    let repository = the_repository(&workspaces);
    assert_eq!(repository.id, opened.repository);
    assert_eq!(repository.name, "repo");
    assert_eq!(repository.git_dir, repo.join(".git"));
    let [main, feat] = &repository.workspaces[..] else {
        panic!("not two worktrees: {:?}", repository.workspaces);
    };
    assert_eq!(
        main,
        &Workspace {
            id: opened.workspace.clone(),
            path: repo.clone(),
            head: branch("main", &first),
        }
    );
    assert_eq!((&feat.path, &feat.head), (&linked, &branch("feat", &first)));
    assert_ne!(feat.id, main.id);

    // Opening the second gives the workspace that was listed, in the same repository,
    // and the list is what it was.
    assert_eq!(
        workspaces.open_folder(&linked).expect("open the folder"),
        Opened {
            repository: repository.id.clone(),
            workspace: feat.id.clone(),
        }
    );
    assert_eq!(the_repository(&workspaces), repository);

    // A branch switched outside the app shows at the next listing.
    git(&linked, &["switch", "--quiet", "-c", "feature/login"]);
    assert_eq!(
        the_repository(&workspaces).workspaces[1].head,
        branch("feature/login", &first)
    );
}

// WS-08.
#[test]
fn a_folder_that_holds_a_repository_or_a_worktree_becomes_a_workspace_and_any_other_is_refused_with_the_reason()
 {
    let scratch = Scratch::new();
    let (repo, linked, _) = a_repository_with_two_worktrees(&scratch);
    let plain = scratch.join("plain/folder");
    std::fs::create_dir_all(&plain).expect("a plain folder");
    let workspaces = workspaces(&scratch);

    // Refused, with the reason, and nothing comes of it.
    let refusal = refused(workspaces.open_folder(&plain));
    assert_eq!(refusal, Refusal::NotARepository(plain.clone()));
    assert_eq!(
        refusal.to_string(),
        format!(
            "{} holds no git repository and no worktree",
            plain.display()
        )
    );
    assert_eq!(workspaces.repositories().expect("repositories"), vec![]);

    // A folder that holds a repository.
    let of_the_repository = workspaces.open_folder(&repo).expect("open the folder");
    // A folder that holds a worktree of it.
    let of_the_worktree = workspaces.open_folder(&linked).expect("open the folder");
    assert_eq!(of_the_worktree.repository, of_the_repository.repository);
    assert_ne!(of_the_worktree.workspace, of_the_repository.workspace);
    // A folder inside one is the worktree it lies in.
    assert_eq!(
        workspaces
            .open_folder(&repo.join("src/deep"))
            .expect("open the folder"),
        of_the_repository
    );

    // Each is a workspace: listed at its folder, and its changes can be asked for.
    let listed = the_repository(&workspaces).workspaces;
    let at = |id| {
        let found: &Workspace = listed.iter().find(|w| &w.id == id).expect("listed");
        found.path.clone()
    };
    assert_eq!(at(&of_the_repository.workspace), repo);
    assert_eq!(at(&of_the_worktree.workspace), linked);
    for opened in [&of_the_repository, &of_the_worktree] {
        workspaces.changes(&opened.workspace).expect("changes");
    }
}

#[test]
fn what_cannot_be_a_workspace_is_refused_each_with_its_own_reason() {
    let scratch = Scratch::new();
    let (repo, _, _) = a_repository_with_two_worktrees(&scratch);
    git(
        scratch.path(),
        &["clone", "--quiet", "--bare", "repo", "bare.git"],
    );
    let workspaces = workspaces(&scratch);
    let refusal = |folder: PathBuf| refused(workspaces.open_folder(&folder));

    let nowhere = scratch.join("nowhere");
    assert_eq!(refusal(nowhere.clone()), Refusal::NoSuchFolder(nowhere));
    let a_file = repo.join("src/deep/a.txt");
    assert_eq!(refusal(a_file.clone()), Refusal::NoSuchFolder(a_file));
    let bare = scratch.join("bare.git");
    assert_eq!(refusal(bare.clone()), Refusal::BareRepository(bare));
    let inside_git = repo.join(".git");
    assert_eq!(
        refusal(inside_git.clone()),
        Refusal::NotARepository(inside_git)
    );
    assert_eq!(workspaces.repositories().expect("repositories"), vec![]);
}

#[test]
fn the_id_of_a_workspace_is_written_once_into_the_git_folder_of_its_worktree_and_nowhere_git_looks()
{
    let scratch = Scratch::new();
    let (repo, linked, _) = a_repository_with_two_worktrees(&scratch);
    let workspaces = workspaces(&scratch);
    let opened = workspaces.open_folder(&repo).expect("open the folder");
    let listed = the_repository(&workspaces).workspaces;

    // Each worktree has its own, in its own git folder.
    assert_eq!(id_in(&repo.join(".git")), opened.workspace.as_str());
    assert_eq!(
        id_in(&repo.join(".git/worktrees/linked")),
        listed[1].id.as_str()
    );
    // Nothing else was left beside them, and nothing that git reports.
    for git_dir in [repo.join(".git"), repo.join(".git/worktrees/linked")] {
        let left: Vec<_> = std::fs::read_dir(&git_dir)
            .expect("read the git folder")
            .map(|entry| entry.expect("an entry").file_name())
            .filter(|name| name.to_string_lossy().starts_with(ID_FILE))
            .collect();
        assert_eq!(left, [ID_FILE]);
    }
    for folder in [&repo, &linked] {
        assert_eq!(git(folder, &["status", "--porcelain", "--ignored"]), "");
    }

    // Opening and listing again, also by a Keeper that restarted, gives the same ids
    // and writes nothing.
    let written = std::fs::metadata(repo.join(".git").join(ID_FILE))
        .expect("the id file")
        .modified()
        .expect("its time");
    assert_eq!(workspaces.open_folder(&repo).expect("open again"), opened);
    drop(workspaces);
    let restarted = support::workspaces(&scratch);
    assert_eq!(the_repository(&restarted).workspaces, listed);
    assert_eq!(restarted.open_folder(&repo).expect("open again"), opened);
    assert_eq!(
        std::fs::metadata(repo.join(".git").join(ID_FILE))
            .expect("the id file")
            .modified()
            .expect("its time"),
        written
    );
}

#[test]
fn a_worktree_made_or_removed_outside_the_app_shows_at_the_next_listing() {
    let scratch = Scratch::new();
    let (repo, linked, first) = a_repository_with_two_worktrees(&scratch);
    let workspaces = workspaces(&scratch);
    workspaces.open_folder(&repo).expect("open the folder");
    let before = the_repository(&workspaces);

    git(
        &repo,
        &["worktree", "add", "--quiet", "--detach", "../third"],
    );
    let with_a_third = the_repository(&workspaces);
    assert_eq!(with_a_third.workspaces[..2], before.workspaces[..]);
    assert_eq!(with_a_third.workspaces.len(), 3);
    assert_eq!(with_a_third.workspaces[2].path, scratch.join("third"));
    assert_eq!(
        with_a_third.workspaces[2].head,
        Head::Detached(yaar_workspaces::CommitId::new(&first))
    );

    // One removed through git, one whose folder was deleted: neither is listed, and
    // neither can be asked for its changes any more.
    git(&repo, &["worktree", "remove", "../third"]);
    std::fs::remove_dir_all(&linked).expect("delete the folder");
    assert_eq!(
        the_repository(&workspaces).workspaces,
        before.workspaces[..1]
    );
    for gone in [&with_a_third.workspaces[2], &before.workspaces[1]] {
        assert_eq!(
            refused(workspaces.changes(&gone.id)),
            Refusal::UnknownWorkspace(gone.id.clone())
        );
    }
}

#[test]
fn a_repository_that_cannot_be_read_is_listed_with_no_worktree_and_the_others_whole() {
    let scratch = Scratch::new();
    let a_repository = |name: &str| {
        let folder = scratch.join(name);
        init(&folder);
        write(&folder, "a.txt", "a\n");
        commit_all(&folder, "first");
        folder
    };
    let unreadable = a_repository("unreadable");
    let (repo, _, _) = a_repository_with_two_worktrees(&scratch);
    let unnamed = a_repository("unnamed");
    let workspaces = workspaces(&scratch);
    for folder in [&unreadable, &repo, &unnamed] {
        workspaces.open_folder(folder).expect("open the folder");
    }
    let whole = workspaces.repositories().expect("repositories");
    let worktrees: Vec<usize> = whole.iter().map(|r| r.workspaces.len()).collect();
    assert_eq!(worktrees, [1, 2, 1]);

    // In the first, git fails on a configuration it cannot read. In the last, the id of
    // the worktree can be neither read nor written: a folder lies where the file was.
    let config = unreadable.join(".git/config");
    let sound = std::fs::read(&config).expect("read the configuration");
    std::fs::write(&config, "[core\n").expect("break the configuration");
    let id_file = unnamed.join(".git").join(ID_FILE);
    std::fs::remove_file(&id_file).expect("remove the id file");
    std::fs::create_dir(&id_file).expect("a folder in its place");

    // Each of the two is still listed, with no worktree; the one between them is listed
    // as it was.
    let without = |repository: &Repository| Repository {
        workspaces: vec![],
        ..repository.clone()
    };
    assert_eq!(
        workspaces.repositories().expect("repositories"),
        [without(&whole[0]), whole[1].clone(), without(&whole[2])]
    );
    // Asked for by itself, a workspace of such a repository says what is wrong.
    let failure = workspaces
        .changes(&whole[0].workspaces[0].id)
        .expect_err("git fails there");
    assert!(
        matches!(&failure, WorkspaceError::Git(_)) && failure.to_string().contains("config"),
        "{failure}"
    );

    // Mended, the repository is listed as it was before.
    std::fs::write(&config, sound).expect("mend the configuration");
    assert_eq!(
        workspaces.repositories().expect("repositories")[..2],
        whole[..2]
    );

    // That git cannot be run at all is the failure of no repository: it is told.
    drop(workspaces);
    let no_git = yaar_recorder::Git::at(scratch.join("nowhere/git"));
    let workspaces = support::workspaces_through(&scratch, no_git);
    assert!(matches!(
        workspaces.repositories(),
        Err(WorkspaceError::Git(yaar_recorder::GitError::NotFound))
    ));
}

#[test]
fn a_repository_that_was_moved_is_the_same_repository_with_the_same_workspaces_and_bases() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "a\n");
    let first = commit_all(&repo, "first");
    git(&repo, &["switch", "--quiet", "-c", "feat"]);
    write(&repo, "a.txt", "b\n");
    commit_all(&repo, "second");

    let workspaces = workspaces(&scratch);
    let opened = workspaces.open_folder(&repo).expect("open the folder");
    workspaces
        .set_base(&opened.workspace, &first)
        .expect("set base");
    let base = workspaces.base(&opened.workspace).expect("base");

    let moved = scratch.join("moved/elsewhere");
    std::fs::create_dir_all(moved.parent().unwrap()).expect("make the folder");
    std::fs::rename(&repo, &moved).expect("move the repository");
    // Until it is opened where it is now, the repository is listed with no worktree.
    let gone = the_repository(&workspaces);
    assert_eq!(
        (gone.id.clone(), gone.workspaces),
        (opened.repository.clone(), vec![])
    );

    assert_eq!(
        workspaces.open_folder(&moved).expect("open the folder"),
        opened
    );
    let repository = the_repository(&workspaces);
    assert_eq!(repository.id, opened.repository);
    assert_eq!(repository.name, "elsewhere");
    assert_eq!(repository.git_dir, moved.join(".git"));
    assert_eq!(repository.workspaces.len(), 1);
    assert_eq!(repository.workspaces[0].id, opened.workspace);
    assert_eq!(repository.workspaces[0].path, moved);
    assert_eq!(workspaces.base(&opened.workspace).expect("base"), base);
    assert_eq!(
        workspaces
            .changes(&opened.workspace)
            .expect("changes")
            .commits
            .len(),
        1
    );
}

#[test]
fn a_linked_worktree_that_was_moved_is_the_same_workspace_with_or_without_gits_knowing() {
    let scratch = Scratch::new();
    let (repo, linked, _) = a_repository_with_two_worktrees(&scratch);
    let workspaces = workspaces(&scratch);
    let opened = workspaces.open_folder(&linked).expect("open the folder");

    // Moved by git: the next listing has it at its new folder.
    git(&repo, &["worktree", "move", "../linked", "../moved-by-git"]);
    let listed = the_repository(&workspaces).workspaces;
    assert_eq!(listed[1].id, opened.workspace);
    assert_eq!(listed[1].path, scratch.join("moved-by-git"));
    workspaces.changes(&opened.workspace).expect("changes");

    // Moved by hand: git still lists the old folder, as one it would forget. Opened
    // where it is, it is the same workspace, and it is listed there.
    let by_hand = scratch.join("moved-by-hand");
    std::fs::rename(scratch.join("moved-by-git"), &by_hand).expect("move the worktree");
    assert_eq!(the_repository(&workspaces).workspaces.len(), 1);
    assert_eq!(
        workspaces.open_folder(&by_hand).expect("open the folder"),
        opened
    );
    let listed = the_repository(&workspaces).workspaces;
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[1].id, opened.workspace);
    assert_eq!(listed[1].path, by_hand);
    workspaces.changes(&opened.workspace).expect("changes");
}

#[test]
fn a_copy_of_a_whole_repository_is_another_repository_with_an_id_of_its_own() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "a\n");
    commit_all(&repo, "first");
    let workspaces = workspaces(&scratch);
    let original = workspaces.open_folder(&repo).expect("open the folder");

    // A copy of the folder takes the id file along.
    let copy = scratch.join("copy");
    let copied = std::process::Command::new("cp")
        .arg("-R")
        .args([&repo, &copy])
        .status()
        .expect("start cp");
    assert!(copied.success());
    assert_eq!(id_in(&copy.join(".git")), original.workspace.as_str());

    let of_the_copy = workspaces.open_folder(&copy).expect("open the folder");
    assert_ne!(of_the_copy.workspace, original.workspace);
    assert_ne!(of_the_copy.repository, original.repository);
    // The new id is written over the copied one; the original keeps its own.
    assert_eq!(id_in(&copy.join(".git")), of_the_copy.workspace.as_str());
    assert_eq!(id_in(&repo.join(".git")), original.workspace.as_str());
    assert_eq!(workspaces.open_folder(&repo).expect("open again"), original);
    assert_eq!(
        workspaces.open_folder(&copy).expect("open again"),
        of_the_copy
    );

    let listed = workspaces.repositories().expect("repositories");
    let paths: Vec<_> = listed
        .iter()
        .map(|repository| (&repository.id, &repository.workspaces[0].path))
        .collect();
    assert_eq!(
        paths,
        [
            (&original.repository, &repo),
            (&of_the_copy.repository, &copy)
        ]
    );
}

#[test]
fn a_copy_of_one_linked_worktree_is_refused_because_it_shares_the_git_folder_of_the_original() {
    let scratch = Scratch::new();
    let (repo, linked, _) = a_repository_with_two_worktrees(&scratch);
    let copy = scratch.join("copy-of-linked");
    let copied = std::process::Command::new("cp")
        .arg("-R")
        .args([&linked, &copy])
        .status()
        .expect("start cp");
    assert!(copied.success());

    // Refused whether or not the original was opened before.
    let workspaces = workspaces(&scratch);
    assert_eq!(
        refused(workspaces.open_folder(&copy)),
        Refusal::CopiedWorktree(copy.clone())
    );
    assert_eq!(workspaces.repositories().expect("repositories"), vec![]);
    let original = workspaces.open_folder(&linked).expect("open the folder");
    assert_eq!(
        refused(workspaces.open_folder(&copy)),
        Refusal::CopiedWorktree(copy.clone())
    );

    // The original is untouched by the attempts.
    assert_eq!(
        id_in(&repo.join(".git/worktrees/linked")),
        original.workspace.as_str()
    );
    assert_eq!(
        workspaces.open_folder(&linked).expect("open again"),
        original
    );
    assert_eq!(the_repository(&workspaces).workspaces.len(), 2);
}

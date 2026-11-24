mod support;

use std::path::Path;

use support::{
    Scratch, a_clone, commit_all, git, git_that_keeps_a_list, init, workspaces, workspaces_through,
    write,
};
use yaar_workspaces::{
    Changes, Comparison, NoComparison, Refusal, WorkspaceError, WorkspaceId, Workspaces,
};

fn same_as_changes(workspaces: &Workspaces, repo: &Path) -> Changes {
    let workspace = workspaces.open_folder(repo).expect("open").workspace;
    same_for(workspaces, &workspace)
}

fn same_for(workspaces: &Workspaces, workspace: &WorkspaceId) -> Changes {
    let changes = workspaces.changes(workspace).expect("changes");
    let (counted_from, commits) = workspaces.counted_from(workspace).expect("counted_from");
    assert_eq!(counted_from, changes.counted_from, "the commit");
    assert_eq!(commits, changes.commits, "the commits");
    changes
}

#[test]
fn with_nothing_committed_there_is_no_commit_to_count_from() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "never committed\n");
    let workspaces = workspaces(&scratch);
    let changes = same_as_changes(&workspaces, &repo);
    assert!(matches!(
        changes.comparison,
        Comparison::NothingToCompare {
            why: NoComparison::NoBase,
            ..
        }
    ));
    assert_eq!(changes.counted_from, None);
}

#[test]
fn on_the_base_branch_with_an_upstream_it_is_the_commit_checked_out_and_the_commits_not_on_the_upstream()
 {
    let scratch = Scratch::new();
    let clone = a_clone(&scratch);
    write(&clone, "a.txt", "three\n");
    commit_all(&clone, "third, not pushed");
    write(&clone, "b.txt", "four\n");
    commit_all(&clone, "fourth, not pushed");
    write(&clone, "uncommitted.txt", "x\n");
    let workspaces = workspaces(&scratch);
    let changes = same_as_changes(&workspaces, &clone);
    assert!(matches!(
        changes.comparison,
        Comparison::NotOnUpstream { .. }
    ));
    assert_eq!(changes.commits.len(), 2);
}

#[test]
fn on_the_base_branch_with_no_upstream_and_a_remote_it_lists_the_commits_on_no_remote_branch() {
    let scratch = Scratch::new();
    let clone = a_clone(&scratch);
    write(&clone, "a.txt", "three\n");
    commit_all(&clone, "third, on no remote");
    git(&clone, &["branch", "--unset-upstream"]);
    let workspaces = workspaces(&scratch);
    let changes = same_as_changes(&workspaces, &clone);
    assert!(matches!(
        changes.comparison,
        Comparison::OnNoRemoteBranch { .. }
    ));
    assert_eq!(changes.commits.len(), 1);
}

#[test]
fn on_the_base_branch_with_no_remote_there_is_nothing_to_compare_and_no_commit_is_listed() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "first");
    write(&repo, "a.txt", "two\n");
    commit_all(&repo, "second");
    let workspaces = workspaces(&scratch);
    let changes = same_as_changes(&workspaces, &repo);
    assert!(matches!(
        changes.comparison,
        Comparison::NothingToCompare {
            why: NoComparison::NoRemote,
            ..
        }
    ));
    assert!(changes.commits.is_empty());
}

#[test]
fn off_the_base_branch_it_is_the_fork_point_and_the_commits_since_and_follows_a_detached_head() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    let first = commit_all(&repo, "first");
    git(&repo, &["switch", "--quiet", "-c", "feat"]);
    write(&repo, "a.txt", "two\n");
    commit_all(&repo, "second");
    write(&repo, "b.txt", "three\n");
    commit_all(&repo, "third");
    let workspaces = workspaces(&scratch);
    let workspace = workspaces.open_folder(&repo).expect("open").workspace;
    let changes = same_for(&workspaces, &workspace);
    assert!(matches!(changes.comparison, Comparison::SinceBase { .. }));
    assert_eq!(
        changes.counted_from.as_ref().map(|c| c.as_str()),
        Some(first.as_str())
    );
    assert_eq!(changes.commits.len(), 2);

    git(&repo, &["switch", "--quiet", "--detach"]);
    write(&repo, "c.txt", "four\n");
    commit_all(&repo, "fourth, on no branch");
    let changes = same_for(&workspaces, &workspace);
    assert!(matches!(changes.comparison, Comparison::SinceBase { .. }));
    assert_eq!(changes.commits.len(), 3);
}

#[test]
fn a_branch_that_shares_no_commit_with_its_base_counts_from_its_own_commit() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "first");
    git(&repo, &["switch", "--quiet", "--orphan", "apart"]);
    write(&repo, "b.txt", "of another history\n");
    let apart = commit_all(&repo, "a history of its own");
    let workspaces = workspaces(&scratch);
    let changes = same_as_changes(&workspaces, &repo);
    assert!(matches!(
        changes.comparison,
        Comparison::NothingToCompare {
            why: NoComparison::NoSharedCommit,
            ..
        }
    ));
    assert_eq!(
        changes.counted_from.as_ref().map(|c| c.as_str()),
        Some(apart.as_str())
    );
}

#[test]
fn a_base_that_git_no_longer_knows_leaves_nothing_to_compare_and_counts_from_the_commit_checked_out()
 {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "first");
    git(&repo, &["branch", "release"]);
    git(&repo, &["switch", "--quiet", "-c", "feat"]);
    write(&repo, "a.txt", "two\n");
    commit_all(&repo, "second");
    let workspaces = workspaces(&scratch);
    let workspace = workspaces.open_folder(&repo).expect("open").workspace;
    workspaces
        .set_base(&workspace, "release")
        .expect("set base");
    git(&repo, &["branch", "--quiet", "-D", "release"]);
    let changes = same_for(&workspaces, &workspace);
    assert!(matches!(
        changes.comparison,
        Comparison::NothingToCompare {
            why: NoComparison::BaseGone,
            ..
        }
    ));
}

#[test]
fn asking_where_the_changes_count_from_starts_no_git_that_reads_the_files() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "first");
    git(&repo, &["switch", "--quiet", "-c", "feat"]);
    write(&repo, "a.txt", "two\n");
    commit_all(&repo, "second");
    write(&repo, "uncommitted.txt", "x\n");

    let (program, list) = git_that_keeps_a_list(&scratch);
    let workspaces = workspaces_through(&scratch, yaar_recorder::Git::at(program));
    let workspace = workspaces.open_folder(&repo).expect("open").workspace;
    // Whatever opening and the first reads started is not what is asked about.
    workspaces.counted_from(&workspace).expect("counted_from");
    let before = std::fs::read_to_string(&list)
        .expect("the list")
        .lines()
        .count();
    for _ in 0..3 {
        workspaces.counted_from(&workspace).expect("counted_from");
    }
    let list = std::fs::read_to_string(&list).expect("the list");
    let started: Vec<&str> = list.lines().skip(before).collect();
    for line in &started {
        for reads_files in ["status", "diff", "ls-files"] {
            assert!(
                !line.split(' ').any(|word| word == reads_files),
                "git was started to read the files: {line}"
            );
        }
    }
}

#[test]
fn a_workspace_the_store_does_not_hold_is_refused() {
    let scratch = Scratch::new();
    let workspaces = workspaces(&scratch);
    let unknown = WorkspaceId::new("0".repeat(32));
    match workspaces.counted_from(&unknown) {
        Err(WorkspaceError::Refused(Refusal::UnknownWorkspace(id))) => assert_eq!(id, unknown),
        other => panic!("not refused: {other:?}"),
    }
}

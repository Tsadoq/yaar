//! Component tests: the base of a workspace and the changes counted from it, through the
//! calls the Keeper makes, in real repositories the tests make and with a real database.

mod support;

use std::path::{Path, PathBuf};

use support::{Scratch, a_clone, commit_all, git, init, workspaces, write};
use yaar_workspaces::{
    Base, BaseSource, Change, ChangedPath, Changes, Commit, CommitId, Comparison, NoComparison,
    Refusal, WorkspaceError, WorkspaceId, Workspaces,
};

fn open(workspaces: &Workspaces, folder: &Path) -> WorkspaceId {
    workspaces
        .open_folder(folder)
        .expect("open the folder")
        .workspace
}

fn default_base(name: &str) -> Base {
    Base {
        name: name.to_owned(),
        source: BaseSource::Default,
    }
}

fn set_by_the_user(name: &str) -> Base {
    Base {
        name: name.to_owned(),
        source: BaseSource::User,
    }
}

fn commit(id: &str, summary: &str) -> Commit {
    Commit {
        id: CommitId::new(id),
        summary: summary.to_owned(),
    }
}

fn changed(path: &str, change: Change) -> ChangedPath {
    ChangedPath {
        path: PathBuf::from(path),
        change,
    }
}

/// The ids git itself lists for these arguments of `rev-list`, newest first.
fn rev_list(repo: &Path, args: &[&str]) -> Vec<String> {
    git(repo, &[&["rev-list"], args].concat())
        .lines()
        .map(str::to_owned)
        .collect()
}

fn ids(commits: &[Commit]) -> Vec<String> {
    commits.iter().map(|commit| commit.id.to_string()).collect()
}

fn refused(result: Result<impl std::fmt::Debug, WorkspaceError>) -> Refusal {
    match result {
        Err(WorkspaceError::Refused(refusal)) => refusal,
        other => panic!("not refused: {other:?}"),
    }
}

// WS-02.
#[test]
fn with_nothing_set_the_base_is_the_default_branch_and_the_changes_count_from_where_the_branch_left_it()
 {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    write(&repo, "kept.txt", "kept\n");
    commit_all(&repo, "first");
    write(&repo, "a.txt", "two\n");
    let left_at = commit_all(&repo, "second");

    git(&repo, &["switch", "--quiet", "-c", "feat"]);
    write(&repo, "a.txt", "three\n");
    let third = commit_all(&repo, "third, on the branch");
    write(&repo, "b.txt", "new\n");
    let fourth = commit_all(&repo, "fourth, on the branch");
    write(&repo, "c.txt", "not committed\n");

    let workspaces = workspaces(&scratch);
    let workspace = open(&workspaces, &repo);
    let expected = Changes {
        comparison: Comparison::SinceBase {
            base: default_base("main"),
        },
        counted_from: Some(CommitId::new(&left_at)),
        commits: vec![
            commit(&fourth, "fourth, on the branch"),
            commit(&third, "third, on the branch"),
        ],
        files: vec![
            changed("a.txt", Change::Modified),
            changed("b.txt", Change::Added),
            changed("c.txt", Change::Added),
        ],
    };
    assert_eq!(
        workspaces.base(&workspace).expect("base"),
        Some(default_base("main"))
    );
    assert_eq!(workspaces.changes(&workspace).expect("changes"), expected);

    // The default branch moves on. The changes still count from the point where the
    // branch left it, not from where it is now: nothing of what `main` gained shows.
    git(&repo, &["switch", "--quiet", "main"]);
    std::fs::remove_file(repo.join("c.txt")).expect("carry nothing over");
    write(&repo, "kept.txt", "changed on main\n");
    write(&repo, "main-only.txt", "never on the branch\n");
    let moved_to = commit_all(&repo, "main moves on");
    git(&repo, &["switch", "--quiet", "feat"]);
    write(&repo, "c.txt", "not committed\n");

    assert_ne!(moved_to, left_at);
    assert_eq!(git(&repo, &["merge-base", "HEAD", "main"]), left_at);
    assert_eq!(
        workspaces.base(&workspace).expect("base"),
        Some(default_base("main"))
    );
    assert_eq!(workspaces.changes(&workspace).expect("changes"), expected);
}

#[test]
fn in_a_clone_the_default_base_is_the_branch_the_remote_recorded_and_not_the_local_copy_of_it() {
    let scratch = Scratch::new();
    let clone = a_clone(&scratch);
    // The remote moves on and is fetched; the local `main` stays where it was, and the
    // branch is made from the remote's.
    let pusher = scratch.join("pusher");
    let remote = scratch.join("remote.git");
    git(
        scratch.path(),
        &["clone", "--quiet", remote.to_str().unwrap(), "pusher"],
    );
    write(&pusher, "a.txt", "three\n");
    let on_the_remote = commit_all(&pusher, "third, pushed by somebody else");
    git(&pusher, &["push", "--quiet", "origin", "main"]);
    git(&clone, &["fetch", "--quiet"]);
    git(
        &clone,
        &[
            "switch",
            "--quiet",
            "--no-track",
            "-c",
            "feat",
            "origin/main",
        ],
    );
    write(&clone, "b.txt", "new\n");
    let on_the_branch = commit_all(&clone, "fourth, on the branch");

    let workspaces = workspaces(&scratch);
    let workspace = open(&workspaces, &clone);
    assert_eq!(
        workspaces.changes(&workspace).expect("changes"),
        Changes {
            comparison: Comparison::SinceBase {
                base: default_base("origin/main"),
            },
            counted_from: Some(CommitId::new(&on_the_remote)),
            commits: vec![commit(&on_the_branch, "fourth, on the branch")],
            files: vec![changed("b.txt", Change::Added)],
        }
    );
}

// WS-04.
#[test]
fn a_base_the_user_set_to_a_branch_or_a_commit_replaces_the_default_and_outlives_the_keeper() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    let first = commit_all(&repo, "first");
    git(&repo, &["branch", "release"]);
    write(&repo, "a.txt", "two\n");
    let second = commit_all(&repo, "second");
    git(&repo, &["switch", "--quiet", "-c", "feat"]);
    write(&repo, "b.txt", "new\n");
    let third = commit_all(&repo, "third, on the branch");
    write(&repo, "c.txt", "new too\n");
    let fourth = commit_all(&repo, "fourth, on the branch");

    let keeper = workspaces(&scratch);
    let workspace = open(&keeper, &repo);
    let since_main = Changes {
        comparison: Comparison::SinceBase {
            base: default_base("main"),
        },
        counted_from: Some(CommitId::new(&second)),
        commits: vec![
            commit(&fourth, "fourth, on the branch"),
            commit(&third, "third, on the branch"),
        ],
        files: vec![
            changed("b.txt", Change::Added),
            changed("c.txt", Change::Added),
        ],
    };
    assert_eq!(keeper.changes(&workspace).expect("changes"), since_main);

    // To a branch: it replaces the default, in the base and in what the changes count
    // from.
    let since_release = Changes {
        comparison: Comparison::SinceBase {
            base: set_by_the_user("release"),
        },
        counted_from: Some(CommitId::new(&first)),
        commits: vec![
            commit(&fourth, "fourth, on the branch"),
            commit(&third, "third, on the branch"),
            commit(&second, "second"),
        ],
        files: vec![
            changed("a.txt", Change::Modified),
            changed("b.txt", Change::Added),
            changed("c.txt", Change::Added),
        ],
    };
    assert_eq!(
        keeper.set_base(&workspace, "release").expect("set base"),
        set_by_the_user("release")
    );
    assert_eq!(
        keeper.base(&workspace).expect("base"),
        Some(set_by_the_user("release"))
    );
    assert_eq!(keeper.changes(&workspace).expect("changes"), since_release);

    // The Keeper restarts: every Workspaces and every Store is dropped, and new ones
    // are opened on the same database file.
    drop(keeper);
    let keeper = workspaces(&scratch);
    assert_eq!(
        keeper.base(&workspace).expect("base"),
        Some(set_by_the_user("release"))
    );
    assert_eq!(keeper.changes(&workspace).expect("changes"), since_release);

    // To a commit, written the way a user writes one: it is kept as the commit it names
    // now.
    let since_third = Changes {
        comparison: Comparison::SinceBase {
            base: set_by_the_user(&third),
        },
        counted_from: Some(CommitId::new(&third)),
        commits: vec![commit(&fourth, "fourth, on the branch")],
        files: vec![changed("c.txt", Change::Added)],
    };
    assert_eq!(
        keeper.set_base(&workspace, "HEAD~1").expect("set base"),
        set_by_the_user(&third)
    );
    assert_eq!(
        keeper.base(&workspace).expect("base"),
        Some(set_by_the_user(&third))
    );
    assert_eq!(keeper.changes(&workspace).expect("changes"), since_third);

    drop(keeper);
    let keeper = workspaces(&scratch);
    assert_eq!(
        keeper.base(&workspace).expect("base"),
        Some(set_by_the_user(&third))
    );
    assert_eq!(keeper.changes(&workspace).expect("changes"), since_third);

    // The base stays on the commit the user saw: after one more commit, `HEAD~1` would
    // be another.
    write(&repo, "d.txt", "later\n");
    let fifth = commit_all(&repo, "fifth, on the branch");
    let changes = keeper.changes(&workspace).expect("changes");
    assert_eq!(changes.counted_from, Some(CommitId::new(&third)));
    assert_eq!(ids(&changes.commits), [fifth, fourth]);
}

// WS-06.
#[test]
fn on_its_base_branch_with_an_upstream_a_workspace_shows_the_commits_not_on_the_upstream_and_what_is_uncommitted()
 {
    let scratch = Scratch::new();
    let clone = a_clone(&scratch);
    write(&clone, "a.txt", "three\n");
    let third = commit_all(&clone, "third, not pushed");
    write(&clone, "b.txt", "new, and committed\n");
    let fourth = commit_all(&clone, "fourth, not pushed");
    write(&clone, "a.txt", "four, not committed\n");
    write(&clone, "untracked.txt", "not committed either\n");

    let workspaces = workspaces(&scratch);
    let workspace = open(&workspaces, &clone);
    let not_on_the_upstream = |base: Base| Changes {
        comparison: Comparison::NotOnUpstream {
            base,
            upstream: "origin/main".to_owned(),
        },
        counted_from: Some(CommitId::new(&fourth)),
        commits: vec![
            commit(&fourth, "fourth, not pushed"),
            commit(&third, "third, not pushed"),
        ],
        // What is not committed, and nothing of the two commits: `b.txt` is theirs.
        files: vec![
            changed("a.txt", Change::Modified),
            changed("untracked.txt", Change::Added),
        ],
    };

    // The base is the branch the remote recorded, and `main` is its local twin.
    assert_eq!(
        workspaces.base(&workspace).expect("base"),
        Some(default_base("origin/main"))
    );
    let changes = workspaces.changes(&workspace).expect("changes");
    assert_eq!(changes, not_on_the_upstream(default_base("origin/main")));
    assert_eq!(
        ids(&changes.commits),
        rev_list(&clone, &["HEAD", "--not", "origin/main"])
    );

    // With no default recorded by the remote the base is `main` itself, and the
    // workspace is on it all the same.
    git(&clone, &["remote", "set-head", "origin", "-d"]);
    assert_eq!(
        workspaces.changes(&workspace).expect("changes"),
        not_on_the_upstream(default_base("main"))
    );

    // Pushed, the commits are on the upstream; what is not committed stays.
    git(&clone, &["push", "--quiet", "origin", "main"]);
    let changes = workspaces.changes(&workspace).expect("changes");
    assert_eq!(changes.commits, vec![]);
    assert_eq!(changes.files.len(), 2);
}

// BASE-01.
#[test]
fn on_a_base_branch_with_no_upstream_and_a_remote_exactly_the_commits_on_no_remote_branch_are_listed()
 {
    let scratch = Scratch::new();
    let clone = a_clone(&scratch);
    // One commit that reaches the remote under another branch, then two that reach no
    // remote at all. `main` follows nothing.
    write(&clone, "a.txt", "three\n");
    let on_a_remote_branch = commit_all(&clone, "third, pushed to another branch");
    git(&clone, &["push", "--quiet", "origin", "main:topic"]);
    write(&clone, "a.txt", "four\n");
    let fourth = commit_all(&clone, "fourth, on no remote");
    write(&clone, "b.txt", "new, and committed\n");
    let fifth = commit_all(&clone, "fifth, on no remote");
    git(&clone, &["branch", "--unset-upstream"]);
    write(&clone, "untracked.txt", "not committed\n");

    let workspaces = workspaces(&scratch);
    let workspace = open(&workspaces, &clone);
    let on_no_remote_branch = |base: Base| Changes {
        comparison: Comparison::OnNoRemoteBranch { base },
        counted_from: Some(CommitId::new(&fifth)),
        commits: vec![
            commit(&fifth, "fifth, on no remote"),
            commit(&fourth, "fourth, on no remote"),
        ],
        files: vec![changed("untracked.txt", Change::Added)],
    };

    let changes = workspaces.changes(&workspace).expect("changes");
    assert_eq!(changes, on_no_remote_branch(default_base("origin/main")));
    // Exactly those: git's own list of what is on no branch of any remote. The third
    // commit is not on `origin/main` and is left out, because `origin/topic` has it.
    assert_eq!(
        ids(&changes.commits),
        rev_list(&clone, &["HEAD", "--not", "--remotes"])
    );
    assert_eq!(
        rev_list(&clone, &["HEAD", "--not", "origin/main"]).len(),
        3,
        "{on_a_remote_branch} is not on origin/main"
    );

    // The same with no default recorded by the remote: the base is `main` itself.
    git(&clone, &["remote", "set-head", "origin", "-d"]);
    assert_eq!(
        workspaces.changes(&workspace).expect("changes"),
        on_no_remote_branch(default_base("main"))
    );
}

// BASE-02. The offer to set a base is, as far as Workspaces makes it, the comparison
// that says there is nothing to compare with and why; the line that offers is the
// pane's.
#[test]
fn with_no_remote_the_uncommitted_changes_are_listed_with_nothing_to_compare_and_a_base_can_be_set()
{
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    let first = commit_all(&repo, "first");
    write(&repo, "b.txt", "new, and committed\n");
    let second = commit_all(&repo, "second");
    write(&repo, "a.txt", "two, not committed\n");
    write(&repo, "untracked.txt", "not committed either\n");

    let workspaces = workspaces(&scratch);
    let workspace = open(&workspaces, &repo);
    assert_eq!(
        workspaces.changes(&workspace).expect("changes"),
        Changes {
            comparison: Comparison::NothingToCompare {
                base: Some(default_base("main")),
                why: NoComparison::NoRemote,
            },
            counted_from: Some(CommitId::new(&second)),
            // Not the whole history of a repository that has nowhere to push to.
            commits: vec![],
            files: vec![
                changed("a.txt", Change::Modified),
                changed("untracked.txt", Change::Added),
            ],
        }
    );

    // The offer, taken: with a base set there is something to compare with.
    workspaces.set_base(&workspace, &first).expect("set base");
    assert_eq!(
        workspaces.changes(&workspace).expect("changes"),
        Changes {
            comparison: Comparison::SinceBase {
                base: set_by_the_user(&first),
            },
            counted_from: Some(CommitId::new(&first)),
            commits: vec![commit(&second, "second")],
            files: vec![
                changed("a.txt", Change::Modified),
                changed("b.txt", Change::Added),
                changed("untracked.txt", Change::Added),
            ],
        }
    );
}

#[test]
fn a_set_base_is_kept_by_the_name_of_its_branch_or_as_the_full_id_of_its_commit() {
    let scratch = Scratch::new();
    let clone = a_clone(&scratch);
    let second = git(&clone, &["rev-parse", "HEAD"]);
    let first = git(&clone, &["rev-parse", "HEAD~1"]);
    git(&clone, &["branch", "release", &first]);
    git(&clone, &["tag", "-a", "-m", "a release", "v1", &first]);
    git(&clone, &["switch", "--quiet", "-c", "feat"]);

    let workspaces = workspaces(&scratch);
    let workspace = open(&workspaces, &clone);
    let kept = |text: &str| {
        workspaces
            .set_base(&workspace, text)
            .expect("set base")
            .name
    };

    // A branch, local or of a remote, however it is written: followed by its name.
    assert_eq!(kept("release"), "release");
    assert_eq!(kept("origin/main"), "origin/main");
    assert_eq!(kept("refs/heads/release"), "release");
    assert_eq!(kept("heads/release"), "release");
    assert_eq!(kept("refs/remotes/origin/main"), "origin/main");
    assert_eq!(kept("remotes/origin/main"), "origin/main");
    // Anything else: the commit it names now, by its full id.
    assert_eq!(kept("v1"), first);
    assert_eq!(kept(&second[..10]), second);
    assert_eq!(kept("HEAD"), second);
    assert_eq!(kept("release~0"), first);

    // What names nothing is refused, and leaves the base that was set.
    for text in ["no-such-branch", "", "--all", "release..feat"] {
        assert_eq!(
            refused(workspaces.set_base(&workspace, text)),
            Refusal::UnknownRevision(text.to_owned())
        );
    }
    assert_eq!(
        workspaces.base(&workspace).expect("base"),
        Some(set_by_the_user(&first))
    );
    let unknown = WorkspaceId::new("0".repeat(32));
    assert_eq!(
        refused(workspaces.set_base(&unknown, "main")),
        Refusal::UnknownWorkspace(unknown)
    );
}

#[test]
fn a_set_base_that_git_no_longer_knows_is_still_the_base_and_leaves_nothing_to_compare_with() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "first");
    git(&repo, &["branch", "release"]);
    git(&repo, &["switch", "--quiet", "-c", "feat"]);
    write(&repo, "a.txt", "two\n");
    let second = commit_all(&repo, "second");
    write(&repo, "b.txt", "not committed\n");

    let workspaces = workspaces(&scratch);
    let workspace = open(&workspaces, &repo);
    workspaces
        .set_base(&workspace, "release")
        .expect("set base");
    git(&repo, &["branch", "--quiet", "-D", "release"]);

    assert_eq!(
        workspaces.base(&workspace).expect("base"),
        Some(set_by_the_user("release"))
    );
    assert_eq!(
        workspaces.changes(&workspace).expect("changes"),
        Changes {
            comparison: Comparison::NothingToCompare {
                base: Some(set_by_the_user("release")),
                why: NoComparison::BaseGone,
            },
            counted_from: Some(CommitId::new(&second)),
            commits: vec![],
            files: vec![changed("b.txt", Change::Added)],
        }
    );
}

#[test]
fn with_nothing_committed_every_file_is_new_and_there_is_nothing_to_compare_with() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "never committed\n");
    write(&repo, "b.txt", "staged\n");
    git(&repo, &["add", "b.txt"]);

    let workspaces = workspaces(&scratch);
    let workspace = open(&workspaces, &repo);
    assert_eq!(workspaces.base(&workspace).expect("base"), None);
    let nothing_committed = Changes {
        comparison: Comparison::NothingToCompare {
            base: None,
            why: NoComparison::NoBase,
        },
        counted_from: None,
        commits: vec![],
        files: vec![
            changed("a.txt", Change::Added),
            changed("b.txt", Change::Added),
        ],
    };
    assert_eq!(
        workspaces.changes(&workspace).expect("changes"),
        nothing_committed
    );

    // Another branch has commits, and this one, made apart from it, still has none: the
    // base is found, and shares no commit with a branch that has no commit.
    commit_all(&repo, "first");
    git(&repo, &["switch", "--quiet", "--orphan", "apart"]);
    write(&repo, "a.txt", "never committed here\n");
    write(&repo, "b.txt", "nor this\n");
    assert_eq!(
        workspaces.base(&workspace).expect("base"),
        Some(default_base("main"))
    );
    assert_eq!(
        workspaces.changes(&workspace).expect("changes"),
        Changes {
            comparison: Comparison::NothingToCompare {
                base: Some(default_base("main")),
                why: NoComparison::NoSharedCommit,
            },
            ..nothing_committed
        }
    );
}

#[test]
fn a_branch_that_shares_no_commit_with_its_base_has_nothing_to_compare_with() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "first");
    git(&repo, &["switch", "--quiet", "--orphan", "apart"]);
    write(&repo, "b.txt", "of another history\n");
    let apart = commit_all(&repo, "a history of its own");
    write(&repo, "c.txt", "not committed\n");

    let workspaces = workspaces(&scratch);
    let workspace = open(&workspaces, &repo);
    assert_eq!(
        workspaces.changes(&workspace).expect("changes"),
        Changes {
            comparison: Comparison::NothingToCompare {
                base: Some(default_base("main")),
                why: NoComparison::NoSharedCommit,
            },
            counted_from: Some(CommitId::new(&apart)),
            commits: vec![],
            files: vec![changed("c.txt", Change::Added)],
        }
    );
}

#[test]
fn with_no_branch_checked_out_the_changes_count_from_where_the_commit_left_the_base() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    let first = commit_all(&repo, "first");
    git(&repo, &["switch", "--quiet", "--detach"]);
    write(&repo, "a.txt", "two\n");
    let second = commit_all(&repo, "second, on no branch");

    let workspaces = workspaces(&scratch);
    let workspace = open(&workspaces, &repo);
    assert_eq!(
        workspaces.changes(&workspace).expect("changes"),
        Changes {
            comparison: Comparison::SinceBase {
                base: default_base("main"),
            },
            counted_from: Some(CommitId::new(&first)),
            commits: vec![commit(&second, "second, on no branch")],
            files: vec![changed("a.txt", Change::Modified)],
        }
    );
}

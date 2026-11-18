//! Component tests: where a worktree is, what is checked out in it, the branches around
//! it and its commits, read through the surface the rest of the Keeper uses, in real
//! repositories the tests make.

mod support;

use std::path::{Path, PathBuf};

use support::{Scratch, commit_all, git, init, program, write};
use yaar_recorder::{
    Branch, Commit, CommitId, Commits, Git, GitError, Head, Listed, LocalBranch, OpenError, Refs,
    RemoteBranch, Worktree,
};

fn found() -> Git {
    Git::find().expect("git on the search path")
}

fn open(folder: &Path) -> Worktree {
    Worktree::open(&found(), folder).expect("open")
}

fn branch(name: &str, commit: &str) -> Head {
    Head::Branch {
        name: name.to_owned(),
        commit: Some(CommitId::new(commit)),
    }
}

fn local(name: &str, ahead: u32, upstream: Option<&str>) -> LocalBranch {
    LocalBranch {
        name: name.to_owned(),
        ahead,
        upstream: upstream.map(str::to_owned),
    }
}

fn of_origin(name: &str, ahead: u32) -> RemoteBranch {
    RemoteBranch {
        name: name.to_owned(),
        remote: "origin".to_owned(),
        ahead,
    }
}

/// A repository with two commits on `main` and a branch `topic`, a bare copy of it to
/// serve as the remote, and a clone of that. Returns the clone.
fn a_clone(scratch: &Scratch) -> PathBuf {
    let source = scratch.join("source");
    init(&source);
    write(&source, "a.txt", "one\n");
    commit_all(&source, "first");
    write(&source, "a.txt", "two\n");
    commit_all(&source, "second");
    git(&source, &["branch", "topic"]);
    let remote = scratch.join("remote.git");
    git(
        scratch.path(),
        &["clone", "--quiet", "--bare", "source", "remote.git"],
    );
    let clone = scratch.join("clone");
    git(
        scratch.path(),
        &["clone", "--quiet", remote.to_str().unwrap(), "clone"],
    );
    clone
}

#[test]
fn a_folder_opens_as_the_worktree_it_lies_in_with_its_links_resolved() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "src/deep/a.txt", "a\n");
    commit_all(&repo, "start");
    git(
        &repo,
        &["worktree", "add", "--quiet", "-b", "feat", "../linked"],
    );
    std::os::unix::fs::symlink(&repo, scratch.join("a-link")).expect("link");

    let main = open(&repo);
    assert_eq!(main.root(), repo);
    assert_eq!(main.git_dir(), repo.join(".git"));
    assert_eq!(main.common_dir(), repo.join(".git"));

    // Any folder inside is the worktree, and a link to it is the folder it points at.
    assert_eq!(open(&repo.join("src/deep")).root(), repo);
    assert_eq!(open(&scratch.join("a-link")).root(), repo);
    assert_eq!(open(&scratch.join("a-link/src")).root(), repo);

    // A linked worktree has a git folder of its own inside the one all share.
    let linked = open(&scratch.join("linked"));
    assert_eq!(linked.root(), scratch.join("linked"));
    assert_eq!(linked.git_dir(), repo.join(".git/worktrees/linked"));
    assert_eq!(linked.common_dir(), main.common_dir());
}

#[test]
fn what_is_no_worktree_is_refused_with_the_reason() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "a\n");
    commit_all(&repo, "start");
    git(
        scratch.path(),
        &["clone", "--quiet", "--bare", "repo", "bare.git"],
    );
    std::fs::create_dir(scratch.join("plain")).expect("a plain folder");
    let refusal = |folder: &Path| Worktree::open(&found(), folder).expect_err("refused");

    assert!(matches!(
        refusal(&scratch.join("nowhere")),
        OpenError::NoSuchFolder
    ));
    assert!(matches!(
        refusal(&repo.join("a.txt")),
        OpenError::NoSuchFolder
    ));
    assert!(matches!(
        refusal(&scratch.join("plain")),
        OpenError::NotARepository
    ));
    assert!(matches!(
        refusal(&scratch.join("bare.git")),
        OpenError::Bare
    ));
    // Inside the git folder there are no files of the user.
    assert!(matches!(
        refusal(&repo.join(".git")),
        OpenError::NotARepository
    ));
    assert!(matches!(
        refusal(&repo.join(".git/objects")),
        OpenError::NotARepository
    ));
}

#[test]
fn what_is_checked_out_is_a_branch_with_or_without_a_commit_or_a_commit_alone() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    let worktree = open(&repo);
    assert_eq!(
        worktree.head().expect("head"),
        Head::Branch {
            name: "main".to_owned(),
            commit: None
        }
    );

    write(&repo, "a.txt", "a\n");
    let first = commit_all(&repo, "start");
    git(&repo, &["switch", "--quiet", "-c", "feature/login"]);
    assert_eq!(
        worktree.head().expect("head"),
        branch("feature/login", &first)
    );

    git(&repo, &["switch", "--quiet", "--detach"]);
    assert_eq!(
        worktree.head().expect("head"),
        Head::Detached(CommitId::new(&first))
    );
}

#[test]
fn the_branches_around_a_clone_are_listed_with_their_distance_and_what_they_follow() {
    let scratch = Scratch::new();
    let clone = a_clone(&scratch);
    git(
        &clone,
        &["branch", "--quiet", "--track", "follower", "origin/topic"],
    );
    git(&clone, &["switch", "--quiet", "-c", "feat"]);
    write(&clone, "b.txt", "b\n");
    commit_all(&clone, "third");
    write(&clone, "b.txt", "bb\n");
    commit_all(&clone, "fourth");
    // A second remote that was never fetched from has a name and nothing else.
    git(&clone, &["remote", "add", "other", "/nowhere/other.git"]);

    let worktree = open(&clone);
    assert_eq!(
        worktree.refs().expect("refs"),
        Refs {
            remotes: vec!["origin".to_owned(), "other".to_owned()],
            remote_defaults: vec!["origin/main".to_owned()],
            // What is checked out is two commits ahead of every other branch.
            local: vec![
                local("feat", 0, None),
                local("follower", 2, Some("origin/topic")),
                local("main", 2, Some("origin/main")),
            ],
            // `origin/HEAD` is the default the remote recorded, not a branch.
            remote: vec![of_origin("origin/main", 2), of_origin("origin/topic", 2)],
        }
    );

    // The remote's default is moved to a branch, and then that branch goes: the default
    // is still listed as recorded, and the branch that followed it follows nothing.
    git(&clone, &["remote", "set-head", "origin", "topic"]);
    git(&clone, &["update-ref", "-d", "refs/remotes/origin/topic"]);
    assert_eq!(
        worktree.refs().expect("refs"),
        Refs {
            remotes: vec!["origin".to_owned(), "other".to_owned()],
            remote_defaults: vec!["origin/topic".to_owned()],
            local: vec![
                local("feat", 0, None),
                local("follower", 2, None),
                local("main", 2, Some("origin/main")),
            ],
            remote: vec![of_origin("origin/main", 2)],
        }
    );

    // With the default deleted, the remote has recorded none.
    git(&clone, &["remote", "set-head", "origin", "-d"]);
    assert_eq!(
        worktree.refs().expect("refs").remote_defaults,
        Vec::<String>::new()
    );
}

#[test]
fn a_branch_that_is_behind_what_is_checked_out_and_ahead_of_it_counts_only_what_is_ahead() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "a\n");
    commit_all(&repo, "start");
    git(&repo, &["switch", "--quiet", "-c", "feat"]);
    for n in 0..3 {
        write(&repo, "b.txt", &format!("{n}\n"));
        commit_all(&repo, &format!("on feat {n}"));
    }
    git(&repo, &["switch", "--quiet", "main"]);
    write(&repo, "c.txt", "c\n");
    commit_all(&repo, "on main");

    // On `main`, one commit that `feat` lacks; `feat` has three that `main` lacks.
    assert_eq!(
        open(&repo).refs().expect("refs").local,
        vec![local("feat", 1, None), local("main", 0, None)]
    );
    git(&repo, &["switch", "--quiet", "feat"]);
    assert_eq!(
        open(&repo).refs().expect("refs").local,
        vec![local("feat", 0, None), local("main", 3, None)]
    );
}

#[test]
fn with_nothing_committed_there_are_no_branches_and_no_commits_and_nothing_fails() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    git(&repo, &["remote", "add", "origin", "/nowhere/origin.git"]);
    let other = scratch.join("other");
    init(&other);
    write(&other, "a.txt", "a\n");
    let elsewhere = CommitId::new(commit_all(&other, "in another repository"));

    let worktree = open(&repo);
    assert_eq!(
        worktree.refs().expect("refs"),
        Refs {
            remotes: vec!["origin".to_owned()],
            ..Refs::default()
        }
    );
    assert_eq!(
        worktree
            .commits(Commits::OnNoRemoteBranch)
            .expect("commits"),
        vec![]
    );
    assert_eq!(worktree.fork_point(&elsewhere).expect("fork point"), None);
    assert_eq!(worktree.resolve("HEAD").expect("resolve"), None);
    assert_eq!(worktree.resolve("main").expect("resolve"), None);
}

#[test]
fn the_worktrees_of_a_repository_are_listed_the_main_one_first_and_only_those_on_disk() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "a\n");
    let first = commit_all(&repo, "start");
    let add = |args: &[&str]| git(&repo, &[&["worktree", "add", "--quiet"], args].concat());
    add(&["-b", "feat", "../on-feat"]);
    add(&["--detach", "../detached"]);
    add(&["-b", "gone", "../gone"]);
    std::fs::remove_dir_all(scratch.join("gone")).expect("delete the folder");

    let main = Listed {
        path: repo.clone(),
        head: branch("main", &first),
        main: true,
        git_dir: repo.join(".git"),
    };
    let on_feat = Listed {
        path: scratch.join("on-feat"),
        head: branch("feat", &first),
        main: false,
        git_dir: repo.join(".git/worktrees/on-feat"),
    };
    let detached = Listed {
        path: scratch.join("detached"),
        head: Head::Detached(CommitId::new(&first)),
        main: false,
        git_dir: repo.join(".git/worktrees/detached"),
    };

    // Asked from any of them, and asked of the git folder they share, the list is the
    // same. The order of the linked ones is git's, which this test does not fix.
    let by_path = |mut list: Vec<Listed>| {
        list[1..].sort_by(|a, b| a.path.cmp(&b.path));
        list
    };
    let expected = vec![main, detached, on_feat];
    assert_eq!(by_path(open(&repo).list().expect("list")), expected);
    assert_eq!(
        by_path(open(&scratch.join("on-feat")).list().expect("list")),
        expected
    );
    assert_eq!(
        by_path(Worktree::list_of(&found(), &repo.join(".git")).expect("list")),
        expected
    );

    // Each listed git folder is the one the worktree itself reports.
    for listed in &expected {
        assert_eq!(open(&listed.path).git_dir(), listed.git_dir);
    }
}

#[test]
fn a_repository_with_no_files_checked_out_lists_only_its_linked_worktrees() {
    let scratch = Scratch::new();
    let source = scratch.join("source");
    init(&source);
    write(&source, "a.txt", "a\n");
    let first = commit_all(&source, "start");
    git(
        scratch.path(),
        &["clone", "--quiet", "--bare", "source", "bare.git"],
    );
    let bare = scratch.join("bare.git");
    git(
        &bare,
        &["worktree", "add", "--quiet", "../checked-out", "main"],
    );

    let expected = vec![Listed {
        path: scratch.join("checked-out"),
        head: branch("main", &first),
        main: false,
        git_dir: bare.join("worktrees/checked-out"),
    }];
    assert_eq!(
        open(&scratch.join("checked-out")).list().expect("list"),
        expected
    );
    assert_eq!(Worktree::list_of(&found(), &bare).expect("list"), expected);
}

#[test]
fn where_there_is_no_repository_any_more_no_worktree_is_listed() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    std::fs::create_dir(scratch.join("plain")).expect("a plain folder");

    let list = |folder: &str| Worktree::list_of(&found(), &scratch.join(folder)).expect("list");
    assert_eq!(list("nowhere/.git"), vec![]);
    assert_eq!(list("plain"), vec![]);
    assert_eq!(list("repo/.git").len(), 1);
}

#[test]
fn a_name_resolves_to_the_commit_it_names_and_to_nothing_when_git_knows_none() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "a\n");
    let first = commit_all(&repo, "start");
    write(&repo, "a.txt", "b\n");
    let second = commit_all(&repo, "more");
    git(&repo, &["tag", "-a", "-m", "a release", "v1", &first]);
    let worktree = open(&repo);
    let resolve = |name: &str| worktree.resolve(name).expect("resolve");

    for name in ["main", "HEAD", &second, &second[..9]] {
        assert_eq!(resolve(name), Some(CommitId::new(&second)), "{name}");
    }
    for name in ["v1", "HEAD~1", "main^"] {
        assert_eq!(resolve(name), Some(CommitId::new(&first)), "{name}");
    }
    // Unknown names, a name that is no commit, and text git would take for an option.
    for name in ["nope", "", "HEAD^{tree}", "--all", "-h", "a\0b", "HEAD~7"] {
        assert_eq!(resolve(name), None, "{name:?}");
    }
}

#[test]
fn the_commits_of_a_branch_are_listed_newest_first_with_the_first_line_of_their_message() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "a\n");
    let first = commit_all(&repo, "start");
    git(&repo, &["switch", "--quiet", "-c", "feat"]);
    write(&repo, "a.txt", "b\n");
    let second = commit_all(&repo, "A summary\n\nand a body\nof two lines");
    write(&repo, "a.txt", "c\n");
    let third = commit_all(&repo, "Third: with a colon, a tab\tand an umlaut ü");
    let worktree = open(&repo);
    let commit = |id: &str, summary: &str| Commit {
        id: CommitId::new(id),
        summary: summary.to_owned(),
    };
    let of_the_branch = vec![
        commit(&third, "Third: with a colon, a tab\tand an umlaut ü"),
        commit(&second, "A summary"),
    ];

    let since = CommitId::new(&first);
    assert_eq!(
        worktree.commits(Commits::Since(&since)).expect("commits"),
        of_the_branch
    );
    assert_eq!(
        worktree
            .commits(Commits::NotOn(Branch::Local("main")))
            .expect("commits"),
        of_the_branch
    );
    // With no remote, no commit is on a branch of one.
    let mut all = of_the_branch.clone();
    all.push(commit(&first, "start"));
    assert_eq!(
        worktree
            .commits(Commits::OnNoRemoteBranch)
            .expect("commits"),
        all
    );
    // A branch git does not know is a failure, not an empty list.
    assert!(matches!(
        worktree.commits(Commits::NotOn(Branch::Local("no-such-branch"))),
        Err(GitError::Failed { .. })
    ));
}

#[test]
fn a_branch_is_read_as_the_branch_where_a_tag_has_its_name() {
    let scratch = Scratch::new();
    let clone = a_clone(&scratch);
    let second = git(&clone, &["rev-parse", "HEAD"]);
    let first = git(&clone, &["rev-parse", "HEAD~1"]);
    write(&clone, "a.txt", "three\n");
    let third = commit_all(&clone, "third, not pushed");
    // Three tags on the first commit: two named like a branch, one like nothing else.
    for tag in ["main", "origin/main", "v1"] {
        git(&clone, &["tag", tag, &first]);
    }
    let worktree = open(&clone);
    let commit = |id: &str| Some(CommitId::new(id));

    // Written as the user writes it, each name is the tag: that is how git reads it.
    assert_eq!(worktree.resolve("main").expect("resolve"), commit(&first));
    assert_eq!(
        worktree.resolve("origin/main").expect("resolve"),
        commit(&first)
    );
    // Asked for as a branch, it is the branch.
    let commit_of = |branch| worktree.commit_of(branch).expect("the commit of a branch");
    assert_eq!(commit_of(Branch::Local("main")), commit(&third));
    assert_eq!(commit_of(Branch::Remote("origin/main")), commit(&second));
    assert_eq!(commit_of(Branch::Remote("origin/topic")), commit(&second));
    // What is no branch of that kind is none, whatever else has the name.
    for none in [
        Branch::Local("origin/main"),
        Branch::Remote("main"),
        Branch::Local("v1"),
        Branch::Remote("v1"),
        Branch::Local("no-such-branch"),
        Branch::Local(""),
    ] {
        assert_eq!(commit_of(none), None, "{none:?}");
    }

    // The same when a branch is what the commits are not to be on.
    let ids = |which| -> Vec<String> {
        let commits = worktree.commits(which).expect("commits");
        commits.iter().map(|commit| commit.id.to_string()).collect()
    };
    assert_eq!(
        ids(Commits::NotOn(Branch::Remote("origin/main"))),
        std::slice::from_ref(&third)
    );
    assert_eq!(
        ids(Commits::NotOn(Branch::Local("main"))),
        Vec::<String>::new()
    );
    assert_eq!(
        ids(Commits::Since(&CommitId::new(&first))),
        [third, second],
        "what the tag would have left in"
    );
}

#[test]
fn two_histories_with_no_shared_commit_have_no_fork_point() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "a\n");
    let first = CommitId::new(commit_all(&repo, "start"));
    git(&repo, &["switch", "--quiet", "--orphan", "apart"]);
    write(&repo, "b.txt", "b\n");
    commit_all(&repo, "a history of its own");

    let worktree = open(&repo);
    assert_eq!(worktree.fork_point(&first).expect("fork point"), None);
    git(&repo, &["switch", "--quiet", "main"]);
    assert_eq!(
        worktree.fork_point(&first).expect("fork point"),
        Some(first)
    );
}

#[test]
fn the_folder_says_which_repository_is_read_whatever_the_environment_names() {
    let scratch = Scratch::new();
    let (asked, named) = (scratch.join("asked"), scratch.join("named"));
    init(&asked);
    write(&asked, "a.txt", "a\n");
    let commit_asked = commit_all(&asked, "in the repository that is asked about");
    init(&named);
    write(&named, "b.txt", "b\n");
    commit_all(&named, "in the repository the environment names");
    git(&named, &["switch", "--quiet", "-c", "elsewhere"]);

    // SAFETY: as in `Scratch::new`: this test has a process of its own under
    // `cargo nextest`, and no other thread of it is running.
    unsafe {
        std::env::set_var("GIT_DIR", named.join(".git"));
        std::env::set_var("GIT_WORK_TREE", &named);
    }
    let worktree = open(&asked);
    assert_eq!(worktree.root(), asked);
    assert_eq!(
        worktree.head().expect("head"),
        branch("main", &commit_asked)
    );
    assert_eq!(
        worktree
            .content_at(&CommitId::new(&commit_asked), Path::new("a.txt"))
            .expect("content"),
        Some(b"a\n".to_vec())
    );
}

#[test]
fn git_is_found_on_the_search_path_and_its_absence_is_told() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    let (first, second) = (scratch.join("first"), scratch.join("second"));
    // In the first folder a file named git that cannot be run; in the second a program.
    write(&first, "git", "not a program\n");
    program(&second.join("git"), "exit 0");
    let search = |folders: &[&Path]| {
        let joined = std::env::join_paths(folders).expect("a search path");
        // SAFETY: as in `Scratch::new`: this test has a process of its own under
        // `cargo nextest`, and no other thread of it is running.
        unsafe { std::env::set_var("PATH", joined) };
    };

    search(&[Path::new("relative"), &first, &second]);
    assert_eq!(Git::find().expect("found").program(), second.join("git"));

    search(&[&first]);
    assert!(matches!(Git::find(), Err(GitError::NotFound)));

    // A program that is not there is told when it is first run.
    let missing = Git::at(scratch.join("nowhere/git"));
    assert_eq!(missing.program(), scratch.join("nowhere/git"));
    assert!(matches!(
        Worktree::open(&missing, &repo),
        Err(OpenError::Git(GitError::NotFound))
    ));
}

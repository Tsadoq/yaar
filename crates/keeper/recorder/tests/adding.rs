//! Component tests: a worktree made from a branch by the user's own git, in real
//! repositories the tests make, through the calls the Workspaces part makes.

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;

use support::{Scratch, an_older_git, commit_all, git, init, program, write};
use yaar_recorder::{AddError, Added, CommitId, Git, Worktree};

fn open(folder: &Path) -> Worktree {
    Worktree::open(&Git::find().expect("git on the search path"), folder).expect("open")
}

/// Everything the user's repository holds that a refusal must leave as it was: the seven
/// reads of the plan.
#[derive(Debug, PartialEq, Eq)]
struct Reads {
    status: Vec<u8>,
    refs: Vec<u8>,
    worktrees: Vec<u8>,
    stash: Vec<u8>,
    unreachable: Vec<u8>,
    index: Vec<u8>,
    config: Vec<u8>,
    objects: Vec<(PathBuf, u64)>,
}

/// What a test reads of the repository and of the folder it lies in.
#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    reads: Reads,
    // Every name below the folder the repository lies in, with its kind.
    listing: Vec<(PathBuf, char)>,
}

fn read_git(repo: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(args)
        .output()
        .expect("start git");
    let mut all = output.stdout;
    all.extend_from_slice(b"|");
    all.extend_from_slice(&output.status.code().unwrap_or(-1).to_le_bytes());
    all
}

fn walk(folder: &Path, below: &Path, found: &mut Vec<(PathBuf, char)>) {
    for entry in std::fs::read_dir(folder).expect("read the folder") {
        let path = entry.expect("an entry").path();
        let kind = std::fs::symlink_metadata(&path).expect("kind").file_type();
        let letter = if kind.is_symlink() {
            'l'
        } else if kind.is_dir() {
            'd'
        } else {
            'f'
        };
        found.push((path.strip_prefix(below).expect("below").to_owned(), letter));
        if letter == 'd' {
            walk(&path, below, found);
        }
    }
}

fn snapshot(repo: &Path, lair: &Path) -> Snapshot {
    let mut objects = Vec::new();
    let mut files = Vec::new();
    walk(
        &repo.join(".git/objects"),
        &repo.join(".git/objects"),
        &mut files,
    );
    for (path, letter) in files {
        if letter == 'f' {
            let size = std::fs::metadata(repo.join(".git/objects").join(&path))
                .expect("size")
                .len();
            objects.push((path, size));
        }
    }
    objects.sort();
    let mut listing = Vec::new();
    walk(lair, lair, &mut listing);
    listing.sort();
    Snapshot {
        reads: Reads {
            status: read_git(
                repo,
                &[
                    "status",
                    "--porcelain=v2",
                    "--ignored",
                    "--untracked-files=all",
                ],
            ),
            refs: read_git(repo, &["for-each-ref"]),
            worktrees: read_git(repo, &["worktree", "list", "--porcelain"]),
            stash: read_git(repo, &["stash", "list"]),
            unreachable: read_git(repo, &["fsck", "--unreachable", "--no-reflogs"]),
            index: std::fs::read(repo.join(".git/index")).expect("the list of files"),
            config: std::fs::read(repo.join(".git/config")).expect("the configuration"),
            objects,
        },
        listing,
    }
}

/// A repository `lair/repo` with `main` at two commits and a file `a.txt`, and two
/// objects no ref reaches: a blob and a dangling commit.
struct Place {
    lair: PathBuf,
    repo: PathBuf,
    planted: Vec<String>,
}

fn place(scratch: &Scratch) -> Place {
    let lair = scratch.join("lair");
    let repo = lair.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "first");
    write(&repo, "a.txt", "two\n");
    commit_all(&repo, "second");

    write(scratch.path(), "blob.txt", "a blob nothing refers to\n");
    let blob = git(
        &repo,
        &[
            "hash-object",
            "-w",
            scratch.join("blob.txt").to_str().expect("text"),
        ],
    );
    write(&repo, "a.txt", "three\n");
    let dangling = git(&repo, &["stash", "create"]);
    git(&repo, &["checkout", "--quiet", "--", "a.txt"]);
    assert!(!dangling.is_empty(), "a dangling commit was made");
    Place {
        lair,
        repo,
        planted: vec![blob, dangling],
    }
}

impl Place {
    fn snapshot(&self) -> Snapshot {
        snapshot(&self.repo, &self.lair)
    }

    /// The folder the app puts worktrees in, beside the repository.
    fn worktrees(&self) -> PathBuf {
        self.lair.join("repo.worktrees")
    }

    fn assert_planted_objects_are_there(&self) {
        for id in &self.planted {
            git(&self.repo, &["cat-file", "-e", id]);
        }
    }
}

fn main_commit(repo: &Path) -> CommitId {
    CommitId::new(git(repo, &["rev-parse", "refs/heads/main"]))
}

/// Calls `add_worktree` for a refusal and says what came back; the barrier of a negative
/// clause is that the call returned.
fn refused(place: &Place, folder: &Path, branch: &str, from: &str) -> AddError {
    let before = place.snapshot();
    let folder_of_worktrees_was_there = place.worktrees().exists();
    let outcome = open(&place.repo).add_worktree(folder, branch, from);
    let after = place.snapshot();
    assert_eq!(before, after, "a refusal changes nothing");
    assert_eq!(
        place.worktrees().exists(),
        folder_of_worktrees_was_there,
        "no folder is made by a refusal"
    );
    place.assert_planted_objects_are_there();
    outcome.expect_err("refused")
}

// A branch name git does not take, one that starts with a dash and the name `@{-1}`, which
// git expands to the branch left last.
#[test]
fn a_name_git_does_not_take_for_a_branch_is_refused_and_nothing_is_made() {
    let scratch = Scratch::new();
    let place = place(&scratch);
    git(&place.repo, &["checkout", "--quiet", "-b", "other"]);
    git(&place.repo, &["checkout", "--quiet", "main"]);
    let folder = place.worktrees().join("x");
    for name in [
        "",
        "has space",
        "a..b",
        "ends.lock",
        "-dash",
        "@{-1}",
        "x/",
        "with\nnewline",
    ] {
        match refused(&place, &folder, name, "main") {
            AddError::NotABranchName(refused) => assert_eq!(refused, name),
            other => panic!("{name:?} was refused as {other:?}"),
        }
    }
}

#[test]
fn a_branch_that_exists_is_refused_checked_out_or_not_and_nothing_is_made() {
    let scratch = Scratch::new();
    let place = place(&scratch);
    git(&place.repo, &["branch", "elsewhere"]);
    let folder = place.worktrees().join("x");
    for name in ["main", "elsewhere"] {
        match refused(&place, &folder, name, "main") {
            AddError::BranchExists(refused) => assert_eq!(refused, name),
            other => panic!("{name:?} was refused as {other:?}"),
        }
    }
}

// A tag is no branch, and a name that is a revision but not a branch name (`main^`,
// `HEAD`, an id) is none either.
#[test]
fn a_start_that_is_no_branch_is_refused_and_nothing_is_made() {
    let scratch = Scratch::new();
    let place = place(&scratch);
    git(&place.repo, &["tag", "release"]);
    let id = git(&place.repo, &["rev-parse", "HEAD"]);
    let folder = place.worktrees().join("x");
    for from in [
        "nowhere", "release", "main^", "main~1", "HEAD", &id, "", "-x",
    ] {
        match refused(&place, &folder, "task", from) {
            AddError::UnknownBranch(refused) => assert_eq!(refused, from),
            other => panic!("{from:?} was refused as {other:?}"),
        }
    }
}

#[test]
fn a_folder_with_anything_at_it_is_refused_and_nothing_is_made() {
    let scratch = Scratch::new();
    let place = place(&scratch);
    std::fs::create_dir_all(place.worktrees()).expect("the folder of worktrees");
    write(&place.worktrees(), "a-file", "in the way\n");
    std::fs::create_dir(place.worktrees().join("a-folder")).expect("a folder");
    std::os::unix::fs::symlink("nowhere", place.worktrees().join("a-link")).expect("a link");
    for name in ["a-file", "a-folder", "a-link"] {
        let folder = place.worktrees().join(name);
        match refused(&place, &folder, "task", "main") {
            AddError::FolderExists(refused) => assert_eq!(refused, folder),
            other => panic!("{name:?} was refused as {other:?}"),
        }
    }
}

#[test]
fn the_user_s_branch_is_made_in_the_folder_as_an_ordinary_worktree_at_the_commit_named() {
    the_user_s_branch_is_made_in_the_folder_as_an_ordinary_worktree_at_the_commit_named_with(
        &Scratch::new(),
    );
}

#[test]
fn the_user_s_branch_is_made_in_the_folder_as_an_ordinary_worktree_at_the_commit_named_on_an_older_git()
 {
    let scratch = Scratch::new();
    an_older_git(&scratch);
    the_user_s_branch_is_made_in_the_folder_as_an_ordinary_worktree_at_the_commit_named_with(
        &scratch,
    );
}

fn the_user_s_branch_is_made_in_the_folder_as_an_ordinary_worktree_at_the_commit_named_with(
    scratch: &Scratch,
) {
    let place = place(scratch);
    let folder = place.worktrees().join("task-one");
    let at = main_commit(&place.repo);
    let config_before = std::fs::read(place.repo.join(".git/config")).expect("config");
    // The user's hook runs, as it does when the user types the command.
    program(
        &place.repo.join(".git/hooks/post-checkout"),
        &format!("echo \"$PWD\" > '{}'", scratch.join("hook-ran").display()),
    );

    let added = open(&place.repo)
        .add_worktree(&folder, "task/one", "main")
        .expect("add");

    assert_eq!(
        added,
        Added {
            folder: folder.clone(),
            branch: "task/one".to_owned(),
            at: at.clone(),
            made_parent: true,
            had_branch_config: false,
        }
    );
    let listed = git(&place.repo, &["worktree", "list", "--porcelain"]);
    assert!(
        listed.contains(&format!(
            "worktree {}\nHEAD {at}\nbranch refs/heads/task/one",
            folder.display()
        )),
        "{listed}"
    );
    assert_eq!(
        std::fs::read_to_string(folder.join("a.txt")).unwrap(),
        "two\n"
    );
    assert!(folder.join(".git").is_file(), "an ordinary linked worktree");
    assert_eq!(git(&folder, &["rev-parse", "HEAD"]), at.as_str());
    assert_eq!(
        git(&folder, &["symbolic-ref", "--short", "HEAD"]),
        "task/one"
    );
    // Nothing is written into the repository beyond what git writes for a worktree: no
    // setting for a branch made from a local one.
    assert_eq!(
        std::fs::read(place.repo.join(".git/config")).expect("config"),
        config_before
    );
    // The hook of the user ran, in the new worktree.
    let ran = std::fs::read_to_string(scratch.join("hook-ran")).expect("the hook ran");
    assert_eq!(Path::new(ran.trim()), folder);

    // A second one finds the folder of worktrees there.
    let second = place.worktrees().join("two");
    let added = open(&place.repo)
        .add_worktree(&second, "two", "main")
        .expect("add");
    assert!(!added.made_parent);
}

// The remote branch has the name `origin/main` in a clone of a second repository.
fn a_clone(scratch: &Scratch) -> Place {
    let source = scratch.join("source");
    init(&source);
    write(&source, "a.txt", "one\n");
    commit_all(&source, "first");
    write(&source, "a.txt", "two\n");
    commit_all(&source, "second");
    let lair = scratch.join("lair");
    std::fs::create_dir_all(&lair).expect("lair");
    git(
        &lair,
        &["clone", "--quiet", source.to_str().expect("text"), "repo"],
    );
    let repo = lair.join("repo");
    git(&repo, &["config", "user.name", "A Test"]);
    git(&repo, &["config", "user.email", "test@example.invalid"]);
    Place {
        lair,
        repo,
        planted: Vec::new(),
    }
}

#[test]
fn a_branch_of_a_remote_as_the_start_makes_the_branch_and_taking_it_away_leaves_the_configuration_as_it_was()
 {
    let scratch = Scratch::new();
    let place = a_clone(&scratch);
    let at = CommitId::new(git(&place.repo, &["rev-parse", "refs/remotes/origin/main"]));
    let before = place.snapshot();
    let folder = place.worktrees().join("from-remote");

    let worktree = open(&place.repo);
    let added = worktree
        .add_worktree(&folder, "from-remote", "origin/main")
        .expect("add");

    assert_eq!(added.at, at);
    assert!(!added.had_branch_config);
    assert_eq!(git(&folder, &["rev-parse", "HEAD"]), at.as_str());
    // git set the remote branch as the upstream, as it does for the user.
    assert_eq!(
        git(&place.repo, &["config", "branch.from-remote.remote"]),
        "origin"
    );
    assert_ne!(
        std::fs::read(place.repo.join(".git/config")).expect("config"),
        before.reads.config
    );

    worktree.remove_added(&added).expect("take away");

    assert_eq!(place.snapshot(), before);
}

#[test]
fn a_tag_with_the_name_of_the_start_counts_for_nothing() {
    let scratch = Scratch::new();
    let place = place(&scratch);
    let first = git(&place.repo, &["rev-parse", "main~1"]);
    git(&place.repo, &["branch", "dev"]);
    // The tag `dev` names the first commit, the branch `dev` the second.
    git(&place.repo, &["tag", "dev", &first]);
    assert_eq!(git(&place.repo, &["rev-parse", "dev"]), first);
    let folder = place.worktrees().join("from-dev");

    let added = open(&place.repo)
        .add_worktree(&folder, "from-dev", "dev")
        .expect("add");

    let branch_commit = git(&place.repo, &["rev-parse", "refs/heads/dev"]);
    assert_ne!(branch_commit, first);
    assert_eq!(added.at.as_str(), branch_commit);
    assert_eq!(git(&folder, &["rev-parse", "HEAD"]), branch_commit);
}

#[test]
fn taking_a_worktree_away_leaves_no_folder_no_branch_and_the_repository_as_it_was() {
    let scratch = Scratch::new();
    let place = place(&scratch);
    let before = place.snapshot();
    let folder = place.worktrees().join("task");
    let worktree = open(&place.repo);
    let added = worktree.add_worktree(&folder, "task", "main").expect("add");
    assert!(folder.is_dir());

    worktree.remove_added(&added).expect("take away");

    assert!(!folder.exists());
    assert!(
        !place.worktrees().exists(),
        "the folder the add made is gone"
    );
    let refs = git(&place.repo, &["for-each-ref", "refs/heads"]);
    assert!(!refs.contains("task"), "{refs}");
    assert_eq!(place.snapshot(), before);
    place.assert_planted_objects_are_there();
}

#[test]
fn taking_a_worktree_away_leaves_a_parent_folder_that_it_did_not_make_and_one_that_holds_something()
{
    let scratch = Scratch::new();
    let place = place(&scratch);
    std::fs::create_dir_all(place.worktrees()).expect("the folder of worktrees");
    let worktree = open(&place.repo);
    let one = worktree
        .add_worktree(&place.worktrees().join("one"), "one", "main")
        .expect("add");
    assert!(!one.made_parent);
    worktree.remove_added(&one).expect("take away");
    assert!(place.worktrees().is_dir(), "a folder that was there stays");

    // One the add made, and something of the user's put into it since.
    let other = place.lair.join("fresh");
    let two = worktree
        .add_worktree(&other.join("two"), "two", "main")
        .expect("add");
    assert!(two.made_parent);
    write(&other, "notes.txt", "mine\n");
    worktree.remove_added(&two).expect("take away");
    assert!(other.join("notes.txt").is_file(), "what is inside stays");
}

#[test]
fn a_branch_that_moved_since_is_not_deleted_when_the_worktree_is_taken_away() {
    let scratch = Scratch::new();
    let place = place(&scratch);
    let folder = place.worktrees().join("task");
    let worktree = open(&place.repo);
    let added = worktree.add_worktree(&folder, "task", "main").expect("add");
    write(&folder, "b.txt", "new\n");
    let moved = commit_all(&folder, "work on the branch");

    worktree.remove_added(&added).expect("take away");

    assert!(!folder.exists());
    assert_eq!(git(&place.repo, &["rev-parse", "refs/heads/task"]), moved);
    assert_ne!(moved, added.at.as_str());
}

#[test]
fn a_hook_of_the_user_that_fails_leaves_nothing_and_gives_the_error_of_git() {
    a_hook_of_the_user_that_fails_leaves_nothing_and_gives_the_error_of_git_with(&Scratch::new());
}

#[test]
fn a_hook_of_the_user_that_fails_leaves_nothing_and_gives_the_error_of_git_on_an_older_git() {
    let scratch = Scratch::new();
    an_older_git(&scratch);
    a_hook_of_the_user_that_fails_leaves_nothing_and_gives_the_error_of_git_with(&scratch);
}

fn a_hook_of_the_user_that_fails_leaves_nothing_and_gives_the_error_of_git_with(scratch: &Scratch) {
    let place = place(scratch);
    program(
        &place.repo.join(".git/hooks/post-checkout"),
        &format!(
            "echo ran >> '{}'\nexit 1",
            scratch.join("hook-ran").display()
        ),
    );
    let before = place.snapshot();
    let folder = place.worktrees().join("task");

    let outcome = open(&place.repo).add_worktree(&folder, "task", "main");

    match outcome {
        Err(AddError::Git(_)) => {}
        other => panic!("not a failure of git: {other:?}"),
    }
    // Barrier: the hook ran, so git got as far as the checkout and left what it made.
    assert!(
        scratch.join("hook-ran").is_file(),
        "the hook of the user ran"
    );
    assert_eq!(place.snapshot(), before);
    assert!(!place.worktrees().exists(), "the folder the add made went");
    place.assert_planted_objects_are_there();
}

// A branch `main/x` cannot be made while `main` is one: git fails after the folder was
// made and before it made a worktree.
#[test]
fn a_failure_of_git_before_it_made_the_worktree_leaves_nothing_either() {
    let scratch = Scratch::new();
    let place = place(&scratch);
    let before = place.snapshot();
    let folder = place.worktrees().join("task");

    let outcome = open(&place.repo).add_worktree(&folder, "main/x", "main");

    assert!(matches!(outcome, Err(AddError::Git(_))), "{outcome:?}");
    assert_eq!(place.snapshot(), before);
    assert!(!place.worktrees().exists());
}

// Every read the add makes comes before the parent folder is made: git, asked for the
// configuration's section of the branch, fails after every other check passed, and the
// folder is not there afterwards. The git is the real one, wrapped by a program of the
// test that answers that one question with an error.
#[test]
fn a_read_of_git_that_fails_after_the_other_checks_leaves_no_folder_made() {
    let scratch = Scratch::new();
    let place = place(&scratch);
    let wrapper = scratch.join("bin/git");
    program(
        &wrapper,
        "for argument in \"$@\"; do\n  case \"$argument\" in\n    '^branch'*)\n      echo 'the configuration cannot be read' >&2\n      exit 7 ;;\n  esac\ndone\nexec git \"$@\"",
    );
    let before = place.snapshot();
    let folder = place.worktrees().join("task");
    let worktree = Worktree::open(&Git::at(&wrapper), &place.repo).expect("open");

    let outcome = worktree.add_worktree(&folder, "task", "main");

    // Barrier: the call returned, and it was the read that failed, not a check.
    match outcome {
        Err(AddError::Git(failure)) => assert!(
            failure.to_string().contains("cannot be read"),
            "not the read that failed: {failure}"
        ),
        other => panic!("not a failure of git: {other:?}"),
    }
    assert_eq!(place.snapshot(), before);
    assert!(!place.worktrees().exists(), "no folder was made");
    place.assert_planted_objects_are_there();
}

// Only the folder's parent is made, never its parents: when the parent's own parent is
// not there, no folder is made and git is not run. A call that made both levels would
// leave a folder the user never named whenever the add then failed.
#[test]
fn a_folder_whose_parent_has_no_parent_either_makes_nothing() {
    let scratch = Scratch::new();
    let place = place(&scratch);
    let before = place.snapshot();
    let missing = place.lair.join("missing");
    let folder = missing.join("places").join("task");

    let outcome = open(&place.repo).add_worktree(&folder, "task", "main");

    // Barrier: the call returned, as a failure that is not a refusal of a check.
    assert!(matches!(outcome, Err(AddError::Git(_))), "{outcome:?}");
    assert_eq!(place.snapshot(), before);
    assert!(
        std::fs::symlink_metadata(&missing).is_err(),
        "no folder was made"
    );
    place.assert_planted_objects_are_there();
}

// The checks and git's run hold one lock: of several calls for one branch one makes it and
// the others find it there.
#[test]
fn calls_for_one_branch_at_once_make_it_once_and_the_others_are_refused_cleanly() {
    let scratch = Scratch::new();
    let place = place(&scratch);
    let barrier = std::sync::Barrier::new(4);

    let outcomes: Vec<Result<Added, AddError>> = std::thread::scope(|threads| {
        let started: Vec<_> = (0..4)
            .map(|n| {
                let (place, barrier) = (&place, &barrier);
                threads.spawn(move || {
                    let worktree = open(&place.repo);
                    barrier.wait();
                    worktree.add_worktree(&place.worktrees().join(format!("w{n}")), "task", "main")
                })
            })
            .collect();
        started
            .into_iter()
            .map(|thread| thread.join().expect("a thread"))
            .collect()
    });

    let made = outcomes.iter().filter(|outcome| outcome.is_ok()).count();
    assert_eq!(made, 1, "{outcomes:?}");
    for outcome in outcomes.iter().filter_map(|outcome| outcome.as_ref().err()) {
        assert!(matches!(outcome, AddError::BranchExists(_)), "{outcome:?}");
    }
    let listed = git(&place.repo, &["worktree", "list", "--porcelain"]);
    assert_eq!(
        listed.matches("branch refs/heads/task").count(),
        1,
        "{listed}"
    );
    // Nothing is left of the calls that found it there.
    let folders = std::fs::read_dir(place.worktrees())
        .expect("folder")
        .count();
    assert_eq!(folders, 1);
}

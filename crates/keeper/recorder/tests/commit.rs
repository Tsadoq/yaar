//! Component tests: the commit control, through the surface the rest of the Keeper uses,
//! in real repositories the tests make.

mod support;

use std::path::Path;

use support::{Scratch, commit_all, git, git_bytes, init, program, write};
use yaar_recorder::{CommitError, CommitId, Git, Worktree};

fn open(folder: &Path) -> Worktree {
    Worktree::open(&Git::find().expect("git on the search path"), folder).expect("open")
}

/// A repository with one change of each kind a user may have made and not committed, and
/// a hook that writes a line to `hook-ran` in the scratch folder each time it runs.
fn with_every_kind_of_change(scratch: &Scratch) -> std::path::PathBuf {
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, ".gitignore", "*.log\n");
    write(&repo, "unstaged.txt", "one\n");
    write(&repo, "staged.txt", "one\n");
    write(&repo, "deleted.txt", "one\n");
    write(
        &repo,
        "old-name.txt",
        "a file\nlong enough\nfor git\nto pair it\nwith its new name\n",
    );
    commit_all(&repo, "start");

    write(&repo, "unstaged.txt", "one\ntwo\n");
    write(&repo, "staged.txt", "one\ntwo\n");
    git(&repo, &["add", "staged.txt"]);
    std::fs::remove_file(repo.join("deleted.txt")).expect("delete");
    git(&repo, &["mv", "old-name.txt", "new-name.txt"]);
    write(
        &repo,
        "new-name.txt",
        "a file\nlong enough\nfor git\nto pair it\nwith its newer name\n",
    );
    write(&repo, "dir/untracked.txt", "new\n");
    write(&repo, "noise.log", "ignored\n");
    program(
        &repo.join(".git/hooks/pre-commit"),
        &format!("echo ran >> '{}'", scratch.join("hook-ran").display()),
    );
    repo
}

// REC-03. The expected tree is what the user's own `git add -A` writes, asked of a copy
// of the repository so that the real one is touched only by the call under test.
#[test]
fn the_commit_control_makes_an_ordinary_commit_of_every_uncommitted_change_with_the_message_given()
{
    let scratch = Scratch::new();
    let repo = with_every_kind_of_change(&scratch);
    std::process::Command::new("cp")
        .arg("-R")
        .arg(&repo)
        .arg(scratch.join("copy"))
        .status()
        .expect("copy");
    git(&scratch.join("copy"), &["add", "--all"]);
    let wanted_tree = git(&scratch.join("copy"), &["write-tree"]);
    let head_before = git(&repo, &["rev-parse", "HEAD"]);
    let branch_before = git(&repo, &["symbolic-ref", "--short", "HEAD"]);
    assert_eq!(branch_before, "main");

    let message = "Say what the change does\n\nWith a second paragraph.";
    let id = open(&repo).commit_all(message).expect("commit");

    // An ordinary commit: the one HEAD is, on the branch that was checked out, a child of
    // what was there, by the identity of the user's environment.
    assert_eq!(id, CommitId::new(git(&repo, &["rev-parse", "HEAD"])));
    assert_eq!(git(&repo, &["rev-parse", "HEAD^"]), head_before);
    assert_eq!(
        git(&repo, &["symbolic-ref", "--short", "HEAD"]),
        branch_before
    );
    assert_eq!(git(&repo, &["rev-parse", "main"]), id.as_str());
    assert_eq!(
        git(&repo, &["log", "-1", "--format=%an <%ae> | %cn <%ce>"]),
        "A Test <test@example.invalid> | A Test <test@example.invalid>"
    );
    // The user's hook ran, once, as it does for the user's own commit.
    assert_eq!(
        std::fs::read_to_string(scratch.join("hook-ran")).expect("the hook ran"),
        "ran\n"
    );
    // Every uncommitted change is in it, and nothing git ignores.
    assert_eq!(git(&repo, &["rev-parse", "HEAD^{tree}"]), wanted_tree);
    assert_eq!(
        git(&repo, &["status", "--porcelain", "--untracked-files=all"]),
        ""
    );
    assert!(!git(&repo, &["ls-files"]).lines().any(|f| f == "noise.log"));
    // With the message given, as git writes it: one line end after it.
    assert_eq!(
        git_bytes(&repo, &["log", "-1", "--format=%B"]),
        format!("{message}\n\n").into_bytes()
    );
}

#[test]
fn a_message_with_a_hash_line_and_letters_beyond_ascii_is_kept_as_it_is() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "start");
    write(&repo, "a.txt", "two\n");

    let message = "Fix \"quotes\" and 'ticks'\n\n# not a comment\nSchöne Grüße, 日本語";
    open(&repo).commit_all(message).expect("commit");

    let body = String::from_utf8(git_bytes(&repo, &["log", "-1", "--format=%B"])).unwrap();
    assert_eq!(body.trim_end(), message);
}

#[test]
fn with_nothing_uncommitted_there_is_nothing_to_commit_and_nothing_changes() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, ".gitignore", "*.log\n");
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "start");
    // Ignored files are not changes.
    write(&repo, "noise.log", "ignored\n");
    let head = git(&repo, &["rev-parse", "HEAD"]);
    program(
        &repo.join(".git/hooks/pre-commit"),
        &format!("echo ran >> '{}'", scratch.join("hook-ran").display()),
    );

    let error = open(&repo).commit_all("empty").expect_err("no commit");

    assert!(matches!(error, CommitError::NothingToCommit), "{error:?}");
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head);
    assert!(!scratch.join("hook-ran").exists());
}

#[test]
fn a_hook_that_rejects_the_commit_gives_its_words_and_leaves_the_branch_where_it_was() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "start");
    write(&repo, "a.txt", "two\n");
    let head = git(&repo, &["rev-parse", "HEAD"]);
    program(
        &repo.join(".git/hooks/pre-commit"),
        "echo 'the tests are red' >&2\nexit 1",
    );

    let error = open(&repo).commit_all("try").expect_err("no commit");

    match error {
        CommitError::Refused { stderr } => {
            assert!(stderr.contains("the tests are red"), "{stderr:?}")
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head);
}

#[test]
fn an_empty_message_is_refused_by_git_and_makes_no_commit() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "start");
    write(&repo, "a.txt", "two\n");
    let head = git(&repo, &["rev-parse", "HEAD"]);

    let error = open(&repo).commit_all("").expect_err("no commit");

    assert!(matches!(error, CommitError::Refused { .. }), "{error:?}");
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head);
}

#[test]
fn the_first_commit_of_a_repository_is_made_too() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");

    let id = open(&repo).commit_all("first").expect("commit");

    assert_eq!(id.as_str(), git(&repo, &["rev-parse", "HEAD"]));
    assert_eq!(git(&repo, &["rev-list", "--count", "HEAD"]), "1");
}

mod support;

use std::path::Path;

use support::{Scratch, commit_all, git, git_bytes, init, program, write};
use yaar_recorder::{CommitError, CommitId, Git, Worktree};

fn open(folder: &Path) -> Worktree {
    Worktree::open(&Git::find().expect("git on the search path"), folder).expect("open")
}

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
    assert_eq!(
        std::fs::read_to_string(scratch.join("hook-ran")).expect("the hook ran"),
        "ran\n"
    );
    assert_eq!(git(&repo, &["rev-parse", "HEAD^{tree}"]), wanted_tree);
    assert_eq!(
        git(&repo, &["status", "--porcelain", "--untracked-files=all"]),
        ""
    );
    assert!(!git(&repo, &["ls-files"]).lines().any(|f| f == "noise.log"));
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
fn a_commit_git_refuses_with_its_words_on_its_standard_output_still_gives_them() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    let inner = repo.join("inner");
    init(&inner);
    write(&inner, "b.txt", "one\n");
    commit_all(&inner, "start inner");
    commit_all(&repo, "start");
    // `git add --all` does not stage a change inside a nested repository, so `git commit` finds
    // nothing and says so on stdout.
    write(&inner, "b.txt", "two\n");
    let head = git(&repo, &["rev-parse", "HEAD"]);

    let error = open(&repo).commit_all("try").expect_err("no commit");

    match error {
        CommitError::Refused { stderr } => {
            assert!(stderr.contains("no changes added to commit"), "{stderr:?}")
        }
        other => panic!("{other:?}"),
    }
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

fn with_a_partial_staging(scratch: &Scratch) -> std::path::PathBuf {
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "staged.txt", "one\n");
    write(&repo, "unstaged.txt", "one\n");
    commit_all(&repo, "start");
    write(&repo, "staged.txt", "one\ntwo\n");
    git(&repo, &["add", "staged.txt"]);
    write(&repo, "unstaged.txt", "one\ntwo\n");
    write(&repo, "new.txt", "new\n");
    repo
}

fn with_a_partial_staging_and_a_hook_that_refuses(scratch: &Scratch) -> std::path::PathBuf {
    let repo = with_a_partial_staging(scratch);
    program(
        &repo.join(".git/hooks/pre-commit"),
        "echo refused >&2\nexit 1",
    );
    repo
}

/// The bytes of the list of files, then what `git status` says. Status may rewrite the list,
/// so the list is read first, and `settled` runs status once before the first call.
fn list_and_status(worktree: &Path) -> (Vec<u8>, Vec<u8>) {
    let git_dir = git(worktree, &["rev-parse", "--absolute-git-dir"]);
    let list = std::fs::read(Path::new(&git_dir).join("index")).expect("the list of files");
    let status = git_bytes(
        worktree,
        &["status", "--porcelain=v2", "-z", "--untracked-files=all"],
    );
    (list, status)
}

fn settled(worktree: &Path) -> (Vec<u8>, Vec<u8>) {
    git_bytes(worktree, &["status", "--porcelain=v2", "-z"]);
    list_and_status(worktree)
}

#[test]
fn a_commit_a_hook_refuses_puts_the_list_of_files_back_as_it_was() {
    let scratch = Scratch::new();
    let repo = with_a_partial_staging_and_a_hook_that_refuses(&scratch);
    let (list, status) = settled(&repo);
    let unstaged = std::fs::read(repo.join("unstaged.txt")).expect("read");

    let error = open(&repo).commit_all("try").expect_err("no commit");

    assert!(matches!(error, CommitError::Refused { .. }), "{error:?}");
    let (list_after, status_after) = list_and_status(&repo);
    assert_eq!(list_after, list, "the bytes of the list of files");
    assert_eq!(status_after, status, "what git status says");
    assert_eq!(
        git(&repo, &["diff", "--cached", "--name-only"]),
        "staged.txt"
    );
    assert_eq!(
        std::fs::read(repo.join("unstaged.txt")).expect("read"),
        unstaged
    );
    let left: Vec<String> = std::fs::read_dir(repo.join(".git"))
        .expect("read the git folder")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("index") && name != "index")
        .collect();
    assert!(left.is_empty(), "left in the git folder: {left:?}");
}

#[test]
fn a_commit_refused_with_nothing_staged_leaves_the_list_of_files_as_it_was() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "start");
    write(&repo, "a.txt", "two\n");
    write(&repo, "new.txt", "new\n");
    program(&repo.join(".git/hooks/pre-commit"), "exit 1");
    let (list, status) = settled(&repo);

    let error = open(&repo).commit_all("try").expect_err("no commit");

    assert!(matches!(error, CommitError::Refused { .. }), "{error:?}");
    let (list_after, status_after) = list_and_status(&repo);
    assert_eq!(list_after, list, "the bytes of the list of files");
    assert_eq!(status_after, status, "what git status says");
    assert_eq!(git(&repo, &["diff", "--cached", "--name-only"]), "");
}

#[test]
fn a_commit_refused_in_a_repository_with_no_list_of_files_leaves_none() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    program(&repo.join(".git/hooks/pre-commit"), "exit 1");
    assert!(!repo.join(".git/index").exists(), "the premise");

    let error = open(&repo).commit_all("try").expect_err("no commit");

    assert!(matches!(error, CommitError::Refused { .. }), "{error:?}");
    assert!(!repo.join(".git/index").exists());
}

#[test]
fn a_commit_refused_in_a_linked_worktree_puts_back_the_list_of_that_worktree() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "start");
    let linked = scratch.join("linked");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "feat",
            linked.to_str().unwrap(),
        ],
    );
    write(&linked, "a.txt", "two\n");
    write(&linked, "new.txt", "new\n");
    git(&linked, &["add", "a.txt"]);
    program(&repo.join(".git/hooks/pre-commit"), "exit 1");
    let (list, status) = settled(&linked);

    let error = open(&linked).commit_all("try").expect_err("no commit");

    assert!(matches!(error, CommitError::Refused { .. }), "{error:?}");
    let (list_after, status_after) = list_and_status(&linked);
    assert_eq!(list_after, list, "the bytes of the list of files");
    assert_eq!(status_after, status, "what git status says");
    assert_eq!(git(&linked, &["diff", "--cached", "--name-only"]), "a.txt");
}

#[test]
fn a_commit_git_itself_refuses_puts_the_list_of_files_back_as_it_was() {
    let scratch = Scratch::new();
    let repo = with_a_partial_staging(&scratch);
    let (list, status) = settled(&repo);
    let head = git(&repo, &["rev-parse", "HEAD"]);

    let error = open(&repo).commit_all("").expect_err("no commit");

    assert!(matches!(error, CommitError::Refused { .. }), "{error:?}");
    let (list_after, status_after) = list_and_status(&repo);
    assert_eq!(list_after, list, "the bytes of the list of files");
    assert_eq!(status_after, status, "what git status says");
    assert_eq!(
        git(&repo, &["diff", "--cached", "--name-only"]),
        "staged.txt"
    );
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head);
}

#[test]
fn a_commit_where_git_cannot_add_the_files_leaves_the_list_of_files_as_it_was() {
    let scratch = Scratch::new();
    let repo = with_a_partial_staging(&scratch);
    git(&repo, &["config", "add.ignoreErrors", "true"]);
    init(&repo.join("inner"));
    write(&repo.join("inner"), "i.txt", "inside\n");
    let (list, status) = settled(&repo);
    let unstaged = std::fs::read(repo.join("unstaged.txt")).expect("read");
    let inside = std::fs::read(repo.join("inner/i.txt")).expect("read");

    let error = open(&repo).commit_all("try").expect_err("no commit");

    assert!(matches!(error, CommitError::Refused { .. }), "{error:?}");
    let (list_after, status_after) = list_and_status(&repo);
    assert_eq!(list_after, list, "the bytes of the list of files");
    assert_eq!(status_after, status, "what git status says");
    assert_eq!(
        git(&repo, &["diff", "--cached", "--name-only"]),
        "staged.txt"
    );
    assert_eq!(
        std::fs::read(repo.join("unstaged.txt")).expect("read"),
        unstaged
    );
    assert_eq!(
        std::fs::read(repo.join("inner/i.txt")).expect("read"),
        inside
    );
}

// The hook plays another git that holds the lock and is writing the list: the bytes taken
// before must not overwrite what it writes.
#[test]
fn a_list_of_files_that_another_git_is_writing_is_not_overwritten_by_the_putting_back() {
    let scratch = Scratch::new();
    let repo = with_a_partial_staging(&scratch);
    program(
        &repo.join(".git/hooks/pre-commit"),
        "echo 'the other git' > \"$(git rev-parse --git-dir)/index.lock\"\nexit 1",
    );
    let (list, _) = settled(&repo);

    let error = open(&repo).commit_all("try").expect_err("no commit");

    match error {
        CommitError::Refused { stderr } => {
            assert!(stderr.contains("could not be put back"), "{stderr:?}")
        }
        other => panic!("{other:?}"),
    }
    let (list_after, _) = list_and_status(&repo);
    assert_ne!(list_after, list, "the list was overwritten");
    assert_eq!(
        std::fs::read_to_string(repo.join(".git/index.lock")).expect("the lock is still there"),
        "the other git\n"
    );
}

#[test]
fn a_commit_where_another_git_holds_the_lock_of_the_list_changes_nothing() {
    let scratch = Scratch::new();
    let repo = with_a_partial_staging(&scratch);
    let (list, status) = settled(&repo);
    std::fs::write(repo.join(".git/index.lock"), "the other git\n").expect("take the lock");

    let error = open(&repo).commit_all("try").expect_err("no commit");

    match error {
        CommitError::Refused { stderr } => {
            assert!(stderr.contains("index.lock"), "{stderr:?}");
            assert!(!stderr.contains("could not be put back"), "{stderr:?}");
        }
        other => panic!("{other:?}"),
    }
    std::fs::remove_file(repo.join(".git/index.lock")).expect("the lock is still there");
    let (list_after, status_after) = list_and_status(&repo);
    assert_eq!(list_after, list, "the bytes of the list of files");
    assert_eq!(status_after, status, "what git status says");
}

#[test]
fn a_list_that_the_git_holding_the_lock_wrote_after_our_add_failed_is_left_as_it_wrote_it() {
    let scratch = Scratch::new();
    let repo = with_a_partial_staging(&scratch);
    settled(&repo);
    std::fs::write(repo.join(".git/index.lock"), "the other git\n").expect("take the lock");
    let wrapper = scratch.join("bin/git-other-ends");
    program(
        &wrapper,
        &format!(
            "git \"$@\"\ncode=$?\ncase \" $* \" in\n*' add --all '*)\n\
             rm '{lock}'\ngit -C '{repo}' add new.txt\n;;\nesac\nexit $code",
            lock = repo.join(".git/index.lock").display(),
            repo = repo.display()
        ),
    );
    let worktree = Worktree::open(&Git::at(wrapper), &repo).expect("open");

    let error = worktree.commit_all("try").expect_err("no commit");

    assert!(matches!(error, CommitError::Refused { .. }), "{error:?}");
    assert_eq!(
        git(&repo, &["diff", "--cached", "--name-only"]),
        "new.txt\nstaged.txt",
        "what the other git staged"
    );
}

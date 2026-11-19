//! Component tests: any two versions of a worktree compared, and a file read in any of
//! them, through the surface the rest of the Keeper uses, in real repositories the tests
//! make.

mod support;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use support::{Scratch, commit_all, git, init, write};
use yaar_recorder::{Change, ChangedPath, Git, RecordError, StateStore, Worktree};

fn found() -> Git {
    Git::find().expect("git on the search path")
}

fn open(folder: &Path) -> Worktree {
    Worktree::open(&found(), folder).expect("open")
}

/// A store in the scratch folder, outside every repository, as the app's folder is.
fn store(scratch: &Scratch) -> StateStore {
    StateStore::open(&found(), &scratch.join("app/states/one.git")).expect("open the store")
}

/// A document long enough for git to pair it with a copy that has one line changed.
fn document(last: &str) -> String {
    format!("one\ntwo\nthree\nfour\nfive\nsix\nseven\n{last}\n")
}

/// What a path went through, as one word: git's own letters, a rename with the path it
/// came from.
type Pair = (String, String);

/// The paths and changes of a comparison, as a set, in git's words.
fn as_pairs(changed: &[ChangedPath]) -> BTreeSet<Pair> {
    changed
        .iter()
        .map(|changed| {
            let word = match &changed.change {
                Change::Added => "A".to_owned(),
                Change::Modified => "M".to_owned(),
                Change::Deleted => "D".to_owned(),
                Change::Renamed { from } => format!("R {}", from.display()),
            };
            (
                changed.path.to_str().expect("a path of text").to_owned(),
                word,
            )
        })
        .collect()
}

/// What `git diff --name-status -M -z` printed, read by the test and not by the crate:
/// `M`, `A`, `D` and `T` and their path, and `R` and a score with the old path and the new.
fn pairs_of(printed: &[u8]) -> BTreeSet<Pair> {
    let mut fields = printed
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .map(|field| String::from_utf8(field.to_vec()).expect("a path of text"));
    let mut pairs = BTreeSet::new();
    while let Some(status) = fields.next() {
        let path = fields.next().expect("a path after a status");
        pairs.insert(match status.as_bytes()[0] {
            b'R' => {
                let new = fields.next().expect("a new path after a rename");
                (new, format!("R {path}"))
            }
            b'T' => (path, "M".to_owned()),
            letter => (path, (letter as char).to_string()),
        });
    }
    pairs
}

/// What git's diff of two versions says, asked in the real repository, which reads the
/// objects of a state through the store's folder as a second place for objects: the store
/// the call under test works on is left as it is.
fn git_diff(repo: &Path, store: &StateStore, from: &str, to: &str) -> BTreeSet<Pair> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["diff", "--name-status", "-M", "-z", from, to])
        .env(
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            store.folder().join("objects"),
        )
        .stdin(Stdio::null())
        .output()
        .expect("start git");
    assert!(
        output.status.success(),
        "git diff {from} {to}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    pairs_of(&output.stdout)
}

/// A byte copy of `repo`, in which the user's own git can be asked what it would have
/// done, without touching the real one.
fn byte_copy(scratch: &Scratch, repo: &Path, name: &str) -> PathBuf {
    let copy = scratch.join(name);
    let status = Command::new("cp")
        .arg("-R")
        .arg(repo)
        .arg(&copy)
        .status()
        .expect("copy");
    assert!(status.success(), "cp -R failed");
    copy
}

/// A repository with commits C0 to C3 and states S1 on C1 and S3 on C2, and in it the
/// paths that go through every change a diff names: edited, added, deleted, and a file
/// that moved and was edited on the way.
///
/// C0 holds `doc.txt` and C1 replaces it with `notes.txt`, which is that document with a
/// line changed: a rename git pairs.
struct Lived {
    repo: PathBuf,
    commits: [String; 4],
    states: [String; 2],
}

fn lived(scratch: &Scratch, store: &StateStore) -> Lived {
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "base.txt", "base\n");
    write(&repo, "keep.txt", "keep\n");
    write(&repo, "doc.txt", &document("the original"));
    let c0 = commit_all(&repo, "base");
    let worktree = open(&repo);

    std::fs::remove_file(repo.join("doc.txt")).expect("delete");
    write(&repo, "notes.txt", &document("the notes"));
    write(&repo, "base.txt", "base\none\n");
    write(&repo, "moving.txt", &document("before the move"));
    let c1 = commit_all(&repo, "one");
    write(&repo, "draft.txt", "draft\n");
    let s1 = store.record(&worktree).expect("record S1");

    std::fs::remove_file(repo.join("draft.txt")).expect("delete");
    std::fs::remove_file(repo.join("moving.txt")).expect("delete");
    write(&repo, "moved.txt", &document("after the move"));
    write(&repo, "base.txt", "base\none\ntwo\n");
    let c2 = commit_all(&repo, "two");
    write(&repo, "extra/new.txt", "new\n");
    write(&repo, "keep.txt", "kept\nand changed\n");
    let s3 = store.record(&worktree).expect("record S3");
    let c3 = commit_all(&repo, "three");

    Lived {
        repo,
        commits: [c0, c1, c2, c3],
        states: [s1.id.as_str().to_owned(), s3.id.as_str().to_owned()],
    }
}

// REC-02, the first clause: any two of commits and states, both orders. The expectation is
// what git itself prints for the two trees.
#[test]
fn the_diff_between_any_two_of_commits_and_states_equals_gits_diff_of_the_two_trees() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let lived = lived(&scratch, &store);
    let worktree = open(&lived.repo);
    let [_, c1, c2, _] = &lived.commits;
    let [s1, s3] = &lived.states;
    let versions = [c1, c2, s1, s3];

    let mut renames = 0;
    let mut compared = 0;
    for from in versions {
        for to in versions {
            let given = store
                .compare(Some(&worktree), from, Some(to))
                .expect("compare");
            let expected = git_diff(&lived.repo, &store, from, to);
            assert_eq!(as_pairs(&given), expected, "from {from} to {to}");
            // Ordered by path, as the paths of a diff are.
            let paths: Vec<&PathBuf> = given.iter().map(|changed| &changed.path).collect();
            assert!(paths.is_sorted(), "{paths:?}");
            renames += expected
                .iter()
                .filter(|(_, word)| word.starts_with('R'))
                .count();
            compared += 1;
        }
    }
    assert_eq!(
        compared, 16,
        "every pair, in both orders, and each with itself"
    );
    assert!(
        renames >= 2,
        "the premise: git pairs renames among the pairs, and the comparison does too"
    );
    assert!(
        store
            .compare(Some(&worktree), c1, Some(c1))
            .expect("compare")
            .is_empty(),
        "a version with itself differs in nothing"
    );
}

// REC-02, the second clause: a version and the files on disk, which no commit holds.
#[test]
fn the_diff_between_a_version_and_the_files_on_disk_equals_gits_diff_of_a_commit_of_everything() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let lived = lived(&scratch, &store);
    let worktree = open(&lived.repo);
    // Every kind of change on disk, staged or not: edited, deleted, untracked, a file
    // that moved and was edited, and one that is ignored.
    write(&lived.repo, ".gitignore", "*.log\n");
    write(&lived.repo, "noise.log", "ignored\n");
    write(&lived.repo, "base.txt", "base\nchanged on disk\n");
    std::fs::remove_file(lived.repo.join("keep.txt")).expect("delete");
    write(&lived.repo, "fresh/untracked.txt", "untracked\n");
    std::fs::rename(
        lived.repo.join("moved.txt"),
        lived.repo.join("moved-again.txt"),
    )
    .expect("move");
    write(
        &lived.repo,
        "moved-again.txt",
        &document("after the second move"),
    );
    git(&lived.repo, &["add", "base.txt"]);

    // What git says, in a byte copy: everything added to its list of files and written
    // as a tree, then the diff of the version and that tree, with the states' objects
    // read from the store.
    let copy = byte_copy(&scratch, &lived.repo, "copy");
    git(&copy, &["add", "--all"]);
    let tree = git(&copy, &["write-tree"]);
    let states_before = store.states().expect("states");
    let index_before = std::fs::read(lived.repo.join(".git/index")).expect("the list of files");

    let mut renames = 0;
    for from in lived.commits.iter().skip(1).chain(lived.states.iter()) {
        let given = store.compare(Some(&worktree), from, None).expect("compare");
        let expected = git_diff(&copy, &store, from, &tree);
        assert_eq!(
            as_pairs(&given),
            expected,
            "from {from} to the files on disk"
        );
        renames += expected
            .iter()
            .filter(|(_, word)| word.starts_with('R'))
            .count();
    }
    assert!(renames >= 1, "the premise: a rename is among them");

    assert_eq!(
        store.states().expect("states"),
        states_before,
        "comparing with the files on disk records no state"
    );
    assert_eq!(
        std::fs::read(lived.repo.join(".git/index")).expect("the list of files"),
        index_before,
        "and works on a copy of the list of files, not on the user's"
    );
}

// The files on disk are read as a recording reads them, and a blob they hold that only
// the real repository has is not in the store: git reads it from there to pair a rename.
#[test]
fn a_rename_to_a_file_whose_content_only_the_real_repository_holds_is_paired() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let lived = lived(&scratch, &store);
    let worktree = open(&lived.repo);
    let [_, _, c2, _] = &lived.commits;
    // `doc.txt` as C0 had it: a blob no state and no commit the store holds has.
    std::fs::remove_file(lived.repo.join("notes.txt")).expect("delete");
    write(&lived.repo, "doc.txt", &document("the original"));

    let given = store
        .compare(Some(&worktree), c2, None)
        .expect("compare with the files on disk");

    assert!(
        as_pairs(&given).contains(&("doc.txt".to_owned(), "R notes.txt".to_owned())),
        "{given:?}"
    );
}

#[test]
fn a_commit_that_is_compared_is_kept_and_reads_with_the_repository_gone() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let lived = lived(&scratch, &store);
    let worktree = open(&lived.repo);
    // No state was taken on either: the store has not got them.
    let [c0, _, _, c3] = &lived.commits;
    assert!(
        store.paths(c0).is_err() && store.paths(c3).is_err(),
        "the premise: the store holds neither"
    );
    let expected = git_diff(&lived.repo, &store, c0, c3);

    let given = store
        .compare(Some(&worktree), c0, Some(c3))
        .expect("compare");

    assert_eq!(as_pairs(&given), expected);
    drop(worktree);
    std::fs::remove_dir_all(&lived.repo).expect("delete the repository");
    assert_eq!(
        as_pairs(
            &store
                .compare(None, c0, Some(c3))
                .expect("compare with no worktree")
        ),
        expected,
        "with the repository deleted and no worktree, what was kept is compared"
    );
    assert_eq!(
        store.content(c0, Path::new("doc.txt")).expect("read"),
        Some(document("the original").into_bytes()),
        "and read"
    );
}

#[test]
fn with_no_worktree_only_what_the_store_holds_can_be_compared() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let lived = lived(&scratch, &store);
    let [s1, s3] = &lived.states;
    let [c0, ..] = &lived.commits;

    assert!(
        store.compare(None, s1, Some(s3)).is_ok(),
        "two states need no worktree"
    );
    let not_held = store.compare(None, c0, Some(s3));
    let Err(RecordError::Store { reason, .. }) = not_held else {
        panic!("a commit the store lacks, with no worktree: {not_held:?}");
    };
    assert!(
        reason.contains(c0.as_str()),
        "it names the version: {reason}"
    );
    let no_files = store.compare(None, s1, None);
    assert!(
        matches!(no_files, Err(RecordError::Store { .. })),
        "the files on disk need the worktree: {no_files:?}"
    );
}

#[test]
fn something_that_is_no_commit_is_refused_and_leaves_the_store_as_it_was() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let lived = lived(&scratch, &store);
    let worktree = open(&lived.repo);
    let [_, c1, ..] = &lived.commits;
    let refs_before = git(store.folder(), &["for-each-ref"]);

    for given in ["HEAD", "not-an-id", &"0".repeat(40)] {
        assert!(
            store.compare(Some(&worktree), c1, Some(given)).is_err(),
            "{given}"
        );
        assert!(
            store.compare(Some(&worktree), given, Some(c1)).is_err(),
            "{given}"
        );
        assert!(
            store
                .content_in(Some(&worktree), given, Path::new("a"))
                .is_err()
        );
    }
    assert_eq!(git(store.folder(), &["for-each-ref"]), refs_before);
}

// The commit that is asked first is not kept when the one asked second is refused: a call
// that is an error changes nothing in the store.
#[test]
fn a_comparison_that_is_refused_keeps_neither_of_its_versions() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let lived = lived(&scratch, &store);
    let worktree = open(&lived.repo);
    let never_kept = &lived.commits[3];
    assert!(
        store
            .content_in(None, never_kept, Path::new("base.txt"))
            .is_err(),
        "the premise: the store does not hold it"
    );
    let refs_before = git(store.folder(), &["for-each-ref"]);

    for given in ["HEAD", "not-an-id", &"0".repeat(40)] {
        assert!(
            store
                .compare(Some(&worktree), never_kept, Some(given))
                .is_err(),
            "{given}"
        );
        assert!(
            store
                .compare(Some(&worktree), given, Some(never_kept))
                .is_err(),
            "{given}"
        );
    }

    assert_eq!(git(store.folder(), &["for-each-ref"]), refs_before);
    assert!(
        store
            .content_in(None, never_kept, Path::new("base.txt"))
            .is_err(),
        "still not held"
    );
}

#[test]
fn a_file_reads_in_a_state_in_a_commit_and_as_nothing_where_it_is_not() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let lived = lived(&scratch, &store);
    let worktree = open(&lived.repo);
    let [c0, c1, ..] = &lived.commits;
    let [s1, s3] = &lived.states;
    let read = |version: &str, path: &str| {
        store
            .content_in(Some(&worktree), version, Path::new(path))
            .expect("read")
    };

    assert_eq!(
        read(s1, "draft.txt"),
        Some(b"draft\n".to_vec()),
        "in a state"
    );
    assert_eq!(read(s3, "draft.txt"), None, "the file is not in S3");
    assert_eq!(
        read(c0, "doc.txt"),
        Some(document("the original").into_bytes()),
        "in a commit the store did not hold, which is kept to be read"
    );
    assert_eq!(read(c1, "doc.txt"), None, "the file is not in C1");
    assert_eq!(
        read(c1, "../outside"),
        None,
        "a path outside the worktree is no file"
    );
    drop(worktree);
    assert_eq!(
        store
            .content_in(None, c0, Path::new("doc.txt"))
            .expect("read"),
        Some(document("the original").into_bytes()),
        "once kept, it reads with no worktree"
    );
    assert!(
        store
            .content_in(None, &lived.commits[3], Path::new("base.txt"))
            .is_err(),
        "a commit never kept, with no worktree, is an error and not a file that is not there"
    );
}

//! Component tests: the history of a worktree, its commits and its recorded states, read
//! through the surface the rest of the Keeper uses, in real repositories the tests make.

mod support;

use std::path::Path;

use support::{Scratch, commit_all, git, init, write};
use yaar_recorder::{CommitId, Git, HistoryEntry, RecordedState, StateStore, Worktree, history};

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

fn id(commit: &str) -> CommitId {
    CommitId::new(commit)
}

/// What an entry is, as one word the test can compare: a commit by its id, a state by
/// its id.
fn named(entries: &[HistoryEntry]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| match entry {
            HistoryEntry::Commit(commit) => commit.id.as_str().to_owned(),
            HistoryEntry::State(state) => state.id.as_str().to_owned(),
        })
        .collect()
}

/// Changes a file, so that the next state differs from the one before.
fn change(repo: &Path, path: &str, content: &str) {
    write(repo, path, content);
}

/// A repository whose history is the commits C1, C2 and C3 over a base C0 and the states
/// S1 and S2 on C1 and S3 on C2, in the order `C0 C1 S1 S2 C2 S3 C3`, every commit made in
/// the same second. A branch the test left, with a state of its own, lies beside them.
struct Lived {
    repo: std::path::PathBuf,
    base: String,
    commits: [String; 3],
    states: [RecordedState; 3],
    on_the_branch_left: RecordedState,
}

fn lived(scratch: &Scratch, store: &StateStore) -> Lived {
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "base\n");
    let base = commit_all(&repo, "base");
    let worktree = open(&repo);

    // A branch the user worked on and switched away from: its state is never listed.
    git(&repo, &["checkout", "--quiet", "-b", "left"]);
    change(&repo, "left.txt", "left\n");
    commit_all(&repo, "on the branch left");
    change(&repo, "left.txt", "left again\n");
    let on_the_branch_left = store.record(&worktree).expect("record on the branch left");
    git(&repo, &["checkout", "--quiet", "--force", "main"]);

    change(&repo, "a.txt", "one\n");
    let c1 = commit_all(&repo, "one");
    let s1 = store.record(&worktree).expect("record S1");
    change(&repo, "b.txt", "two\n");
    let s2 = store.record(&worktree).expect("record S2");
    change(&repo, "c.txt", "three\n");
    let c2 = commit_all(&repo, "two");
    change(&repo, "d.txt", "four\n");
    let s3 = store.record(&worktree).expect("record S3");
    let c3 = commit_all(&repo, "three");
    Lived {
        repo,
        base,
        commits: [c1, c2, c3],
        states: [s1, s2, s3],
        on_the_branch_left,
    }
}

// REC-01. Every commit is made in the second the scratch folder fixes, so that only git's
// own order and the numbers of the states can tell what came first.
#[test]
fn the_history_lists_the_commits_and_the_states_in_the_order_they_happened() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let lived = lived(&scratch, &store);
    let worktree = open(&lived.repo);
    let [c1, c2, c3] = &lived.commits;
    let [s1, s2, s3] = &lived.states;
    assert_eq!(
        s1.on.as_ref().map(CommitId::as_str),
        Some(c1.as_str()),
        "the premise: S1 was taken on C1"
    );
    assert_eq!(
        s3.on.as_ref().map(CommitId::as_str),
        Some(c2.as_str()),
        "the premise: S3 was taken on C2"
    );

    let entries = history(&worktree, &store, Some(&id(&lived.base))).expect("the history");

    assert_eq!(
        named(&entries),
        [
            c1.as_str(),
            s1.id.as_str(),
            s2.id.as_str(),
            c2.as_str(),
            s3.id.as_str(),
            c3.as_str()
        ],
        "C1, then the states taken on it, C2, its state, and C3 with nothing after"
    );
    // Each entry says what it is, not only which id it has.
    assert_eq!(
        entries[1],
        HistoryEntry::State(s1.clone()),
        "a state comes with its tree and the commit it was taken on"
    );
    let HistoryEntry::Commit(first) = &entries[0] else {
        panic!("the first entry is a commit: {:?}", entries[0]);
    };
    assert_eq!(first.summary, "one", "a commit comes with its summary");
    assert!(
        !named(&entries).contains(&lived.on_the_branch_left.id.as_str().to_owned()),
        "a state taken on a branch the user switched away from is not listed"
    );
}

#[test]
fn with_no_since_the_history_starts_at_the_first_commit() {
    let scratch = Scratch::new();
    let store = store(&scratch);
    let lived = lived(&scratch, &store);
    let worktree = open(&lived.repo);
    let [c1, c2, c3] = &lived.commits;
    let [s1, s2, s3] = &lived.states;

    let entries = history(&worktree, &store, None).expect("the history");

    assert_eq!(
        named(&entries),
        [
            lived.base.as_str(),
            c1.as_str(),
            s1.id.as_str(),
            s2.id.as_str(),
            c2.as_str(),
            s3.id.as_str(),
            c3.as_str()
        ]
    );
}

#[test]
fn the_states_taken_on_since_itself_come_first() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    let store = store(&scratch);
    init(&repo);
    write(&repo, "a.txt", "base\n");
    let base = commit_all(&repo, "base");
    let worktree = open(&repo);
    change(&repo, "a.txt", "edited on the base\n");
    let on_base = store.record(&worktree).expect("record on the base");
    change(&repo, "a.txt", "committed\n");
    let next = commit_all(&repo, "next");

    let entries = history(&worktree, &store, Some(&id(&base))).expect("the history");

    assert_eq!(named(&entries), [on_base.id.as_str(), next.as_str()]);
    let after = history(&worktree, &store, Some(&id(&next))).expect("the history");
    assert_eq!(
        named(&after),
        Vec::<String>::new(),
        "nothing is after the last commit"
    );
}

#[test]
fn a_worktree_with_no_commit_has_the_states_taken_before_any_alone() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    let store = store(&scratch);
    init(&repo);
    write(&repo, "a.txt", "one\n");
    let worktree = open(&repo);

    assert_eq!(
        named(&history(&worktree, &store, None).expect("the history")),
        Vec::<String>::new(),
        "no commit and no state"
    );
    let first = store.record(&worktree).expect("record");
    change(&repo, "a.txt", "two\n");
    let second = store.record(&worktree).expect("record");
    assert_eq!(first.on, None, "the premise: nothing was committed");

    assert_eq!(
        named(&history(&worktree, &store, None).expect("the history")),
        [first.id.as_str(), second.id.as_str()]
    );

    // The same states come first when the first commit is made later.
    let committed = commit_all(&repo, "first");
    assert_eq!(
        named(&history(&worktree, &store, None).expect("the history")),
        [first.id.as_str(), second.id.as_str(), committed.as_str()]
    );
}

// The commit a review counts from is reached through the second parent of a merge when the
// branch took in its base: a walk along first parents alone would never meet it, and list
// what lies before it.
#[test]
fn the_history_follows_first_parents_and_leaves_out_what_lies_before_since() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    let store = store(&scratch);
    init(&repo);
    write(&repo, "a.txt", "root\n");
    let root = commit_all(&repo, "root");
    git(&repo, &["checkout", "--quiet", "-b", "feature"]);
    write(&repo, "f.txt", "feature one\n");
    let feature_one = commit_all(&repo, "feature one");
    git(&repo, &["checkout", "--quiet", "main"]);
    write(&repo, "m.txt", "main one\n");
    let main_one = commit_all(&repo, "main one");
    write(&repo, "m.txt", "main two\n");
    let main_two = commit_all(&repo, "main two");
    git(&repo, &["checkout", "--quiet", "feature"]);
    git(
        &repo,
        &["merge", "--quiet", "--no-ff", "-m", "take in main", "main"],
    );
    let merge = git(&repo, &["rev-parse", "HEAD"]);
    write(&repo, "f.txt", "feature two\n");
    let feature_two = commit_all(&repo, "feature two");
    let worktree = open(&repo);
    // The commit the branch left main at, now main's own tip.
    let counted_from = worktree
        .fork_point(&id(&main_two))
        .expect("fork point")
        .expect("a fork point");
    assert_eq!(counted_from.as_str(), main_two, "the premise: main's tip");

    let entries = history(&worktree, &store, Some(&counted_from)).expect("the history");

    assert_eq!(
        named(&entries),
        [feature_one.as_str(), merge.as_str(), feature_two.as_str()],
        "the branch's own commits and the merge: the commits of main it took in are not \
         listed, nor the root before the commit the history counts from"
    );
    // Counted from the root, more commits come after it than the first parents hold: those
    // of main the merge took in. They are not listed, and the walk stops at the root.
    let from_the_root = history(&worktree, &store, Some(&id(&root))).expect("the history");
    assert_eq!(
        named(&from_the_root),
        [feature_one.as_str(), merge.as_str(), feature_two.as_str()],
        "first parents only, and nothing before the commit it counts from"
    );
    let all = history(&worktree, &store, None).expect("the history");
    assert_eq!(
        named(&all),
        [
            root.as_str(),
            feature_one.as_str(),
            merge.as_str(),
            feature_two.as_str()
        ],
        "from the root: the first parents only, so main's own commits are not in it"
    );
    assert!(!named(&all).contains(&main_one));
}

#[test]
fn a_since_that_is_no_commit_is_an_error_and_not_an_empty_history() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    let store = store(&scratch);
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "one");
    let worktree = open(&repo);

    assert!(history(&worktree, &store, Some(&id(&"0".repeat(40)))).is_err());
}

//! Component tests: the states of a worktree, recorded through the surface the rest of
//! the Keeper uses, in real repositories the tests make, and read back from the store.

mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use support::{Scratch, commit_all, git, git_bytes, init, program, write};
use yaar_recorder::{
    Change, ChangedPath, CommitId, Git, RecordError, RecordedState, StateStore, Worktree,
};

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

/// Every file of a worktree that git does not ignore in these tests (`*.log`) with its
/// bytes, by its path, in the order of the bytes of the path. A link is its target.
/// Found by walking the folders, so that it is what is on disk and not what git says.
fn files_of(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, folder: &Path, found: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(folder).expect("read the folder") {
            let path = entry.expect("an entry").path();
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            if name == ".git" || name.ends_with(".log") {
                continue;
            }
            let kind = std::fs::symlink_metadata(&path).unwrap().file_type();
            if kind.is_dir() {
                walk(root, &path, found);
            } else {
                let content = if kind.is_symlink() {
                    std::os::unix::ffi::OsStringExt::into_vec(
                        std::fs::read_link(&path).unwrap().into_os_string(),
                    )
                } else {
                    std::fs::read(&path).unwrap()
                };
                let relative = path.strip_prefix(root).unwrap().to_str().unwrap();
                found.insert(relative.to_owned(), content);
            }
        }
    }
    let mut found = BTreeMap::new();
    walk(root, root, &mut found);
    found
}

/// What the store gives for a state: every path with its content.
fn held_by(store: &StateStore, state: &RecordedState) -> BTreeMap<String, Vec<u8>> {
    let paths = store
        .paths(state.id.as_str())
        .expect("the paths of a state");
    // As git orders paths: the bytes.
    let strings: Vec<String> = paths
        .iter()
        .map(|path| path.to_str().expect("a path of text").to_owned())
        .collect();
    let mut sorted = strings.clone();
    sorted.sort();
    assert_eq!(
        strings, sorted,
        "the paths come in the order of their bytes"
    );
    strings
        .into_iter()
        .map(|path| {
            let content = store
                .content(state.id.as_str(), Path::new(&path))
                .expect("read")
                .unwrap_or_else(|| panic!("the state has no content for {path}"));
            (path, content)
        })
        .collect()
}

/// The objects of a repository that nothing reaches.
fn unreachable_objects(repo: &Path) -> Vec<String> {
    let mut found: Vec<String> = git(
        repo,
        &["fsck", "--unreachable", "--no-reflogs", "--no-progress"],
    )
    .lines()
    .map(str::to_owned)
    .collect();
    found.sort();
    found
}

/// A real repository as the user's tools see it. Taken in the two orders the row asks
/// for: `git status` refreshes the list of files and may write it, so the bytes of the
/// list are read before it when the repository is looked at after a recording, and
/// after it when it is looked at before, as nothing of the recording is then to blame.
#[derive(Debug, PartialEq)]
struct Seen {
    status: Vec<u8>,
    index: Vec<u8>,
    refs: String,
    stash: String,
    reflog: String,
    unreachable: Vec<String>,
    counts: String,
    // Every file and folder of the git folder but the list of files, whose bytes are
    // compared on their own: a lock or a temporary file left there stops the user's next
    // `git add`, and no other field sees it.
    git_folder: Vec<String>,
}

impl Seen {
    fn before(repo: &Path) -> Seen {
        let status = status(repo);
        Seen::rest(
            repo,
            std::fs::read(repo.join(".git/index")).expect("the index"),
            status,
        )
    }

    fn after(repo: &Path) -> Seen {
        let index = std::fs::read(repo.join(".git/index")).expect("the index");
        // `rest` runs `git status` last.
        Seen::rest_with_status_last(repo, index)
    }

    fn rest_with_status_last(repo: &Path, index: Vec<u8>) -> Seen {
        let mut seen = Seen::rest(repo, index, Vec::new());
        seen.status = status(repo);
        seen
    }

    fn rest(repo: &Path, index: Vec<u8>, status: Vec<u8>) -> Seen {
        Seen {
            status,
            index,
            refs: git(repo, &["for-each-ref", "--format=%(refname) %(objectname)"]),
            stash: git(repo, &["stash", "list"]),
            reflog: git(repo, &["reflog", "show", "--all", "--format=%gd %H %gs"]),
            unreachable: unreachable_objects(repo),
            counts: git(repo, &["count-objects", "-v"]),
            git_folder: listing(&repo.join(".git")),
        }
    }
}

/// Every path under `root`, with its kind, ordered; the list of files, `index`, left out.
fn listing(root: &Path) -> Vec<String> {
    fn walk(root: &Path, folder: &Path, found: &mut Vec<String>) {
        for entry in std::fs::read_dir(folder).expect("read the folder") {
            let path = entry.expect("an entry").path();
            let kind = std::fs::symlink_metadata(&path).unwrap().file_type();
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            if relative == "index" {
                continue;
            }
            let kind = if kind.is_dir() {
                "folder"
            } else if kind.is_symlink() {
                "link"
            } else {
                "file"
            };
            found.push(format!("{kind} {relative}"));
            if kind == "folder" {
                walk(root, &path, found);
            }
        }
    }
    let mut found = Vec::new();
    walk(root, root, &mut found);
    found.sort();
    found
}

fn status(repo: &Path) -> Vec<u8> {
    git_bytes(
        repo,
        &[
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
            "--ignored",
        ],
    )
}

// STATE-01. A state is made with the user's own git, on a copy of the list of files, and
// everything it writes goes to the store. Any of that going wrong shows in the real
// repository: a blob in its objects, a changed list, a moved ref.
#[test]
fn recording_three_states_leaves_the_real_repository_as_it_was() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, ".gitignore", "*.log\n");
    write(&repo, "a.txt", "one\n");
    write(&repo, "b.txt", "bee\n");
    write(&repo, "stable/deep/s.txt", "never changes\n");
    commit_all(&repo, "first");
    write(&repo, "a.txt", "one\ntwo\n");
    commit_all(&repo, "second");
    git(&repo, &["branch", "other"]);
    // A stash entry, then the changes the user is in the middle of.
    write(&repo, "b.txt", "bee, stashed\n");
    git(&repo, &["stash", "push", "--quiet", "-m", "put aside"]);
    write(&repo, "staged.txt", "staged\n");
    git(&repo, &["add", "staged.txt"]);
    // Staged, and then edited again: the state holds what is on disk.
    write(&repo, "staged.txt", "staged, then edited\n");
    write(&repo, "a.txt", "one\ntwo\nthree\n");
    write(&repo, "untracked.txt", "new\n");
    write(&repo, "noise.log", "ignored\n");

    let worktree = open(&repo);
    let store = store(&scratch);
    let mut recorded = Vec::new();
    for round in 1..=3 {
        match round {
            2 => {
                write(&repo, "a.txt", "one\ntwo\nthree\nfour\n");
                write(&repo, "second.txt", "added before the second state\n");
            }
            3 => {
                write(&repo, "a.txt", "changed once more\n");
                write(&repo, "third.txt", "added before the third state\n");
                std::fs::remove_file(repo.join("b.txt")).expect("delete a tracked file");
            }
            _ => {}
        }
        let before = Seen::before(&repo);
        let state = store.record(&worktree).expect("record");
        let after = Seen::after(&repo);

        assert_eq!(after.status, before.status, "git status, state {round}");
        assert_eq!(after.index, before.index, "the index, state {round}");
        assert_eq!(after.refs, before.refs, "the refs, state {round}");
        assert_eq!(after.stash, before.stash, "the stash, state {round}");
        assert_eq!(after.reflog, before.reflog, "the reflog, state {round}");
        assert_eq!(
            after.unreachable, before.unreachable,
            "the unreachable objects, state {round}"
        );
        // No commit and no object came into the repository.
        assert_eq!(
            after.counts, before.counts,
            "the count of objects, state {round}"
        );
        assert_eq!(
            after.git_folder, before.git_folder,
            "the files of the git folder, state {round}"
        );
        recorded.push((state, files_of(&repo)));
    }

    // The recordings happened, and each holds the files as they were then.
    let states = store.states().expect("states");
    assert_eq!(states.len(), 3);
    for ((state, files), listed) in recorded.iter().zip(&states) {
        assert_eq!(state, listed);
        assert_eq!(&held_by(&store, state), files);
    }
    assert!(
        !files_of(&repo).contains_key("noise.log") && !recorded[0].1.contains_key("noise.log"),
        "an ignored file is in no state"
    );
}

// STATE-05, the experiment's case B: an agent commits between two states, and then the
// commit goes away with its branch.
#[test]
fn every_state_reads_after_its_commit_is_collected_and_after_the_repository_is_deleted() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, ".gitignore", "*.log\n");
    write(&repo, "stable/deep/s.txt", "never changes\n");
    write(&repo, "edited.txt", "base\n");
    commit_all(&repo, "base");
    git(&repo, &["switch", "--quiet", "-c", "agent"]);
    // Staged and then left alone: its blob is in the real repository and in no commit, and
    // the worktree's side of git's status shows nothing for it.
    write(
        &repo,
        "staged-only.txt",
        "only in the user's list of files\n",
    );
    git(&repo, &["add", "staged-only.txt"]);

    let worktree = open(&repo);
    let folder = scratch.join("app/states/case-b.git");
    let store = StateStore::open(&found(), &folder).expect("open the store");

    write(&repo, "edited.txt", "base\nfirst\n");
    write(&repo, "new-one.txt", "one\n");
    write(&repo, "noise.log", "ignored\n");
    let first = store.record(&worktree).expect("record");
    let kept_first = files_of(&repo);

    write(&repo, "edited.txt", "base\nfirst\nsecond\n");
    let second = store.record(&worktree).expect("record");
    let kept_second = files_of(&repo);

    // The agent commits some of it, and goes on. The staged file is gone by then, so that
    // no commit the store copies holds its blob: only the first two states do.
    git(&repo, &["rm", "--quiet", "--force", "staged-only.txt"]);
    write(&repo, "by-the-agent.txt", "committed by the agent\n");
    let agent_commit = commit_all(&repo, "the agent's work");
    write(&repo, "edited.txt", "base\nfirst\nsecond\nthird\n");
    let third = store.record(&worktree).expect("record");
    let kept_third = files_of(&repo);
    assert_eq!(third.on, Some(CommitId::new(&agent_commit)));
    let kept = [
        (first, kept_first),
        (second, kept_second),
        (third, kept_third),
    ];

    git(&repo, &["switch", "--quiet", "--force", "main"]);
    git(&repo, &["branch", "-D", "--quiet", "agent"]);
    git(&repo, &["reflog", "expire", "--expire=now", "--all"]);
    git(&repo, &["gc", "--prune=now", "--quiet"]);
    // The premise: the repository has lost the agent's commit for good.
    assert!(
        std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["cat-file", "-e", &agent_commit])
            .status()
            .expect("start git")
            .code()
            != Some(0),
        "the agent's commit is collected"
    );

    for (state, files) in &kept {
        assert_eq!(&held_by(&store, state), files, "after the collection");
    }

    std::fs::remove_dir_all(&repo).expect("delete the repository");
    // A store opened anew, so that nothing held in memory answers.
    let again = StateStore::open(&found(), &folder).expect("open the store again");
    for (state, files) in &kept {
        assert_eq!(
            &held_by(&again, state),
            files,
            "after the repository is gone"
        );
    }
    assert!(kept[0].1.contains_key("staged-only.txt"));
    assert!(!kept[2].1.contains_key("staged-only.txt"));
    // The store is a whole repository on its own: git finds nothing missing in it, each
    // state's history is the state and the commit it was taken on, and no more.
    git(&folder, &["fsck", "--full", "--no-dangling"]);
    for (state, _) in &kept {
        let mut expected = vec![state.id.to_string()];
        expected.extend(state.on.iter().map(ToString::to_string));
        assert_eq!(
            git(&folder, &["log", "--format=%H", state.id.as_str()])
                .lines()
                .collect::<Vec<_>>(),
            expected,
            "the history of a state"
        );
    }
}

#[test]
fn recording_twice_with_no_change_gives_the_same_state_and_makes_one_ref() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "first");
    write(&repo, "a.txt", "two\n");
    let worktree = open(&repo);
    let store = store(&scratch);

    let first = store.record(&worktree).expect("record");
    let second = store.record(&worktree).expect("record again");

    assert_eq!(first, second);
    assert_eq!(store.states().expect("states"), vec![first]);
    assert_eq!(
        git(store.folder(), &["for-each-ref", "refs/yaar/states/"])
            .lines()
            .count(),
        1
    );
}

#[test]
fn a_state_is_named_by_a_ref_of_ten_digits_and_holds_the_commit_it_was_taken_on() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    let head = commit_all(&repo, "first");
    write(&repo, "a.txt", "two\n");
    let worktree = open(&repo);
    let store = store(&scratch);

    let state = store.record(&worktree).expect("record");

    assert_eq!(state.on, Some(CommitId::new(&head)));
    assert_eq!(
        git(
            store.folder(),
            &["rev-parse", "refs/yaar/states/0000000001"]
        ),
        state.id.as_str()
    );
    assert_eq!(
        git(
            store.folder(),
            &["rev-parse", &format!("{}^{{tree}}", state.id)]
        ),
        state.tree
    );
    // Taken with the app's own name, whatever the environment of the user says.
    assert_eq!(
        git(
            store.folder(),
            &[
                "log",
                "-1",
                "--format=%an <%ae> | %cn <%ce>",
                state.id.as_str()
            ]
        ),
        "YAAR <yaar@localhost.invalid> | YAAR <yaar@localhost.invalid>"
    );
}

#[test]
fn a_state_is_the_tree_git_add_all_would_write_whatever_is_staged() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, ".gitignore", "*.log\n");
    for name in [
        "edited",
        "staged-edit",
        "gone",
        "staged-gone",
        "unstaged-back",
    ] {
        write(&repo, &format!("{name}.txt"), "base\n");
    }
    write(&repo, "dir/inside.txt", "base\n");
    commit_all(&repo, "base");

    write(&repo, "edited.txt", "edited on disk\n");
    // Staged, and the same on disk: git status shows nothing of it on the worktree's side.
    write(&repo, "staged-edit.txt", "staged edit\n");
    git(&repo, &["add", "staged-edit.txt"]);
    std::fs::remove_file(repo.join("gone.txt")).expect("delete");
    // Staged for deletion and still on disk, so that git lists it untracked.
    git(&repo, &["rm", "--cached", "--quiet", "staged-gone.txt"]);
    // Staged edit, edited back on disk to what the commit holds.
    write(&repo, "unstaged-back.txt", "staged\n");
    git(&repo, &["add", "unstaged-back.txt"]);
    write(&repo, "unstaged-back.txt", "base\n");
    write(&repo, "new/deep/untracked.txt", "new\n");
    write(&repo, "with space/and\nline feed.txt", "odd name\n");
    std::os::unix::fs::symlink("edited.txt", repo.join("a-link")).expect("link");
    write(&repo, "noise.log", "ignored\n");
    std::fs::remove_file(repo.join("dir/inside.txt")).expect("delete");

    // What the user's git makes of it, on a copy of the repository.
    std::process::Command::new("cp")
        .arg("-R")
        .arg(&repo)
        .arg(scratch.join("copy"))
        .status()
        .expect("copy");
    git(&scratch.join("copy"), &["add", "--all"]);
    let wanted = git(&scratch.join("copy"), &["write-tree"]);

    let store = store(&scratch);
    let state = store.record(&open(&repo)).expect("record");

    assert_eq!(state.tree, wanted);
    let held = held_by(&store, &state);
    assert!(!held.contains_key("noise.log") && !held.contains_key("dir/inside.txt"));
    assert_eq!(held["a-link"], b"edited.txt");
    assert_eq!(held["with space/and\nline feed.txt"], b"odd name\n");
}

#[test]
fn a_worktree_with_nothing_committed_has_a_state_that_holds_every_file() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, ".gitignore", "*.log\n");
    write(&repo, "a.txt", "one\n");
    write(&repo, "deep/er/b.txt", "two\n");
    write(&repo, "noise.log", "ignored\n");
    // One file staged: git's list holds it already.
    git(&repo, &["add", "a.txt"]);
    let store = store(&scratch);

    let state = store.record(&open(&repo)).expect("record");

    assert_eq!(state.on, None);
    assert_eq!(held_by(&store, &state), files_of(&repo));
    // The next state, with a first commit made, is a child of it.
    commit_all(&repo, "first");
    write(&repo, "a.txt", "changed\n");
    let next = store.record(&open(&repo)).expect("record");
    assert_ne!(next.tree, state.tree);
    assert_eq!(held_by(&store, &next), files_of(&repo));
}

#[test]
fn a_state_number_is_never_reused_after_a_ref_is_taken_away() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "first");
    let worktree = open(&repo);
    let store = store(&scratch);

    write(&repo, "a.txt", "two\n");
    store.record(&worktree).expect("record");
    write(&repo, "a.txt", "three\n");
    store.record(&worktree).expect("record");
    git(
        store.folder(),
        &["update-ref", "-d", "refs/yaar/states/0000000001"],
    );
    write(&repo, "a.txt", "four\n");
    let state = store.record(&worktree).expect("record");

    assert_eq!(
        git(
            store.folder(),
            &["rev-parse", "refs/yaar/states/0000000003"]
        ),
        state.id.as_str(),
        "the third number, not the second"
    );
    assert_eq!(store.states().expect("states").len(), 2);
}

#[test]
fn a_kept_commit_reads_with_the_real_repository_gone_and_is_named_by_a_ref() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    write(&repo, "deep/b.txt", "two\n");
    let old = commit_all(&repo, "old");
    write(&repo, "a.txt", "one, changed\n");
    commit_all(&repo, "newer");
    let worktree = open(&repo);
    let store = store(&scratch);

    store
        .keep_commit(&worktree, &CommitId::new(&old))
        .expect("keep");
    let packs = || {
        std::fs::read_dir(store.folder().join("objects/pack"))
            .unwrap()
            .count()
    };
    let after_one = packs();
    // A commit the store holds already costs nothing.
    store
        .keep_commit(&worktree, &CommitId::new(&old))
        .expect("keep again");
    assert_eq!(packs(), after_one);
    assert_eq!(
        git(
            store.folder(),
            &["rev-parse", &format!("refs/yaar/kept/{old}")]
        ),
        old
    );

    drop(worktree);
    std::fs::remove_dir_all(&repo).expect("delete the repository");
    let again = StateStore::open(&found(), store.folder()).expect("open again");
    assert_eq!(
        again.paths(&old).expect("paths"),
        vec![PathBuf::from("a.txt"), PathBuf::from("deep/b.txt")]
    );
    assert_eq!(
        again.content(&old, Path::new("a.txt")).expect("content"),
        Some(b"one\n".to_vec())
    );
    // A commit that was kept is no state.
    assert_eq!(again.states().expect("states"), vec![]);
}

#[test]
fn a_kept_commit_and_a_state_on_another_head_both_read_when_the_real_repository_is_gone() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "same.txt", "the same\n");
    write(&repo, "moves.txt", "one\n");
    let first = commit_all(&repo, "first");
    write(&repo, "moves.txt", "two\n");
    write(&repo, "added.txt", "added\n");
    commit_all(&repo, "second");
    write(&repo, "moves.txt", "three\n");
    let worktree = open(&repo);
    let store = store(&scratch);

    // The state first, on the second commit; then an older commit, which the store
    // copies from there; then a second state.
    let state = store.record(&worktree).expect("record");
    store
        .keep_commit(&worktree, &CommitId::new(&first))
        .expect("keep");
    write(&repo, "moves.txt", "four\n");
    let next = store.record(&worktree).expect("record");
    let kept = (held_by(&store, &state), held_by(&store, &next));
    let old_files = store.paths(&first).expect("paths");
    assert_eq!(
        old_files,
        vec![PathBuf::from("moves.txt"), PathBuf::from("same.txt")]
    );
    assert_eq!(
        store
            .content(&first, Path::new("moves.txt"))
            .expect("content"),
        Some(b"one\n".to_vec())
    );

    drop(worktree);
    std::fs::remove_dir_all(&repo).expect("delete the repository");
    let again = StateStore::open(&found(), store.folder()).expect("open again");
    assert_eq!((held_by(&again, &state), held_by(&again, &next)), kept);
    assert_eq!(
        again
            .content(&first, Path::new("moves.txt"))
            .expect("content"),
        Some(b"one\n".to_vec())
    );
}

#[test]
fn what_a_state_lacks_is_no_content_and_what_is_not_a_commit_is_refused() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    write(&repo, "dir/b.txt", "two\n");
    commit_all(&repo, "first");
    write(&repo, "a.txt", "two\n");
    let store = store(&scratch);
    let state = store.record(&open(&repo)).expect("record");
    let id = state.id.as_str();

    assert_eq!(
        store.content(id, Path::new("missing.txt")).expect("read"),
        None
    );
    // A folder is no file.
    assert_eq!(store.content(id, Path::new("dir")).expect("read"), None);
    assert_eq!(
        store.content(id, Path::new("../a.txt")).expect("read"),
        None
    );
    assert_eq!(
        store
            .content(&"0".repeat(40), Path::new("a.txt"))
            .expect("read"),
        None
    );
    assert_eq!(
        store.content("HEAD", Path::new("a.txt")).expect("read"),
        None
    );
    assert!(store.paths("--all").is_err());
    assert!(store.changed_paths(id, "not-an-id").is_err());
}

#[test]
fn the_paths_that_differ_between_two_states_come_with_what_happened_to_them() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "edited.txt", "one\n");
    write(&repo, "gone.txt", "soon gone\n");
    write(
        &repo,
        "old-name.txt",
        "a file\nlong enough\nfor git\nto pair it\nwith its new name\n",
    );
    commit_all(&repo, "start");
    let worktree = open(&repo);
    let store = store(&scratch);
    let first = store.record(&worktree).expect("record");

    write(&repo, "edited.txt", "one\ntwo\n");
    std::fs::remove_file(repo.join("gone.txt")).expect("delete");
    std::fs::rename(repo.join("old-name.txt"), repo.join("new-name.txt")).expect("rename");
    write(&repo, "added/a.txt", "new\n");
    let second = store.record(&worktree).expect("record");

    let changed = |path: &str, change: Change| ChangedPath {
        path: PathBuf::from(path),
        change,
    };
    assert_eq!(
        store
            .changed_paths(first.id.as_str(), second.id.as_str())
            .expect("changed"),
        vec![
            changed("added/a.txt", Change::Added),
            changed("edited.txt", Change::Modified),
            changed("gone.txt", Change::Deleted),
            changed(
                "new-name.txt",
                Change::Renamed {
                    from: PathBuf::from("old-name.txt")
                }
            ),
        ]
    );
    assert_eq!(
        store
            .changed_paths(first.id.as_str(), first.id.as_str())
            .expect("changed"),
        vec![]
    );
}

#[test]
fn no_hook_and_no_monitor_of_the_user_runs_when_a_state_is_recorded() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "the same\n");
    commit_all(&repo, "start");
    let worktree = open(&repo);

    // Programs of the user that git starts when its list of files is written or brought
    // up to date, and one that runs when a ref changes.
    let ran = scratch.join("ran");
    let note = |name: &str| format!("echo {name} >> '{}'", ran.display());
    program(
        &repo.join(".git/hooks/post-index-change"),
        &note("index-hook"),
    );
    program(
        &repo.join(".git/hooks/reference-transaction"),
        &note("ref-hook"),
    );
    program(
        &scratch.join("monitor"),
        &format!("{}\nprintf 'token\\0/\\0'", note("monitor")),
    );
    let config = repo.join(".git/config");
    let mut text = std::fs::read_to_string(&config).expect("read the configuration");
    // Written into the file, not through git, which would start the programs itself.
    text.push_str(&format!(
        "[core]\n\tfsmonitor = {}\n",
        scratch.join("monitor").display()
    ));
    std::fs::write(&config, text).expect("write the configuration");
    // The user's configuration reaches a store too, as a global one does.
    let store = store(&scratch);

    write(&repo, "a.txt", "changed\n");
    write(&repo, "b.txt", "new\n");
    store.record(&worktree).expect("record");
    write(&repo, "a.txt", "changed again\n");
    store.record(&worktree).expect("record again");

    assert!(!ran.exists(), "ran: {:?}", std::fs::read_to_string(&ran));
}

// A user who asked git to show the stash in `git status` gets a header record of its own
// in the machine-readable form, which is no file and no reason to fail.
#[test]
fn a_repository_set_to_show_its_stash_in_status_records_as_any_other() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    write(&repo, "b.txt", "bee\n");
    commit_all(&repo, "start");
    write(&repo, "b.txt", "bee, put aside\n");
    git(&repo, &["stash", "push", "--quiet", "-m", "put aside"]);
    // Written into the file, as a user's own setting is.
    let config = repo.join(".git/config");
    let mut text = std::fs::read_to_string(&config).expect("read the configuration");
    text.push_str("[status]\n\tshowStash = true\n");
    std::fs::write(&config, text).expect("write the configuration");
    assert!(
        git(&repo, &["status", "--porcelain=v2", "--branch"]).contains("# stash 1"),
        "the premise: git status prints the stash header"
    );
    let worktree = open(&repo);
    let store = store(&scratch);

    write(&repo, "a.txt", "changed\n");
    write(&repo, "c.txt", "new\n");
    let state = store.record(&worktree).expect("record");

    assert_eq!(held_by(&store, &state), files_of(&repo));
}

#[test]
fn a_linked_worktree_records_its_own_files_and_reads_without_the_repository() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "shared.txt", "committed\n");
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
    write(&repo, "main-only.txt", "only in the main worktree\n");
    write(&linked, "linked-only.txt", "only in the linked one\n");
    write(&linked, "shared.txt", "edited in the linked worktree\n");
    let store = store(&scratch);

    let state = store.record(&open(&linked)).expect("record");
    let kept = held_by(&store, &state);
    assert_eq!(kept, files_of(&linked));
    assert!(!kept.contains_key("main-only.txt"));

    std::fs::remove_dir_all(&repo).expect("delete the repository");
    std::fs::remove_dir_all(&linked).expect("delete the worktree");
    let again = StateStore::open(&found(), store.folder()).expect("open again");
    assert_eq!(held_by(&again, &state), kept);
}

#[test]
fn a_repository_inside_the_worktree_is_recorded_as_the_commit_it_is_on() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "start");
    let inner = repo.join("inner");
    init(&inner);
    write(&inner, "i.txt", "inside\n");
    let inner_head = commit_all(&inner, "inside");
    let store = store(&scratch);

    let state = store.record(&open(&repo)).expect("record");

    assert_eq!(
        git(store.folder(), &["ls-tree", &state.tree, "inner"]),
        format!("160000 commit {inner_head}\tinner")
    );
}

#[test]
fn a_repository_inside_the_worktree_with_no_commit_fails_the_recording_with_gits_words() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "start");
    init(&repo.join("inner"));
    write(&repo.join("inner"), "i.txt", "inside\n");
    let store = store(&scratch);

    let error = store
        .record(&open(&repo))
        .expect_err("a recording that fails");

    assert!(matches!(error, RecordError::Git(_)), "{error:?}");
    assert!(error.to_string().contains("git"), "{error}");
    assert_eq!(store.states().expect("states"), vec![]);
}

#[test]
fn a_store_that_cannot_be_made_says_which_folder_and_why() {
    let scratch = Scratch::new();
    write(
        &scratch.join("app"),
        "states",
        "a file where a folder should be\n",
    );

    let error = StateStore::open(&found(), &scratch.join("app/states/one.git"))
        .expect_err("a store that cannot be made");

    match error {
        RecordError::Store { folder, .. } => assert_eq!(folder, scratch.join("app/states/one.git")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn several_threads_record_one_after_the_other_and_no_git_is_left_running() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "start");
    write(&repo, "a.txt", "changed\n");
    let worktree = open(&repo);
    let store = store(&scratch);

    let states: Vec<RecordedState> = std::thread::scope(|scope| {
        let threads: Vec<_> = (0..4)
            .map(|_| scope.spawn(|| store.record(&worktree).expect("record")))
            .collect();
        threads
            .into_iter()
            .map(|t| t.join().expect("a thread"))
            .collect()
    });

    // The same files: one state, however many asked at once.
    assert!(states.windows(2).all(|pair| pair[0] == pair[1]));
    assert_eq!(store.states().expect("states").len(), 1);
    // No copy of the list of files is left in the temporary folder by this process.
    let own = format!("yaar-index-{}-", std::process::id());
    let left: Vec<_> = std::fs::read_dir(std::env::temp_dir())
        .expect("read the temporary folder")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&own))
        .collect();
    assert!(left.is_empty(), "left behind: {left:?}");
}

#[test]
fn two_stores_that_take_a_state_at_once_never_replace_one_state_with_another() {
    let scratch = Scratch::new();
    // A worktree for each thread, each with files of its own, so that no two recordings
    // find the same tree. Every thread has a handle of its own on the one store folder:
    // the lock of one handle does not hold the others back, as with two processes.
    const THREADS: usize = 8;
    let folder = scratch.join("app/states/one.git");
    StateStore::open(&found(), &folder).expect("make the store");
    let worktrees: Vec<Worktree> = (0..THREADS)
        .map(|n| {
            let repo = scratch.join(&format!("repo{n}"));
            init(&repo);
            write(&repo, "a.txt", "one\n");
            commit_all(&repo, "start");
            write(&repo, "a.txt", &format!("changed in {n}\n"));
            open(&repo)
        })
        .collect();
    let start = std::sync::Barrier::new(THREADS);

    let recorded: Vec<Result<RecordedState, RecordError>> = std::thread::scope(|scope| {
        let threads: Vec<_> = worktrees
            .iter()
            .map(|worktree| {
                let (folder, start) = (&folder, &start);
                scope.spawn(move || {
                    let store = StateStore::open(&found(), folder).expect("open the store");
                    start.wait();
                    store.record(worktree)
                })
            })
            .collect();
        threads
            .into_iter()
            .map(|t| t.join().expect("a thread"))
            .collect()
    });

    // A recording that lost its number says so, and one that says it recorded a state
    // has that state: the store holds every one of them, each under a ref of its own.
    let kept = StateStore::open(&found(), &folder).expect("open the store again");
    let held = kept.states().expect("states");
    let told: Vec<&RecordedState> = recorded.iter().filter_map(|r| r.as_ref().ok()).collect();
    assert!(!told.is_empty(), "no recording succeeded: {recorded:?}");
    for state in &told {
        assert!(
            held.contains(state),
            "a recorded state is not in the store: {state:?} of {held:?}"
        );
    }
    assert_eq!(held.len(), told.len(), "{recorded:?}");
}

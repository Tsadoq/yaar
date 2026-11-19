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
fn the_scratch_folder_drops_the_configuration_a_shell_hands_git_in_its_environment() {
    // SAFETY: changing the environment is sound only while no other thread reads it.
    // This is the first statement of a test, and these tests are run by `cargo
    // nextest`, which gives each test a process of its own.
    unsafe {
        std::env::set_var("GIT_CONFIG_COUNT", "1");
        std::env::set_var("GIT_CONFIG_KEY_0", "alias.status");
        std::env::set_var("GIT_CONFIG_VALUE_0", "log");
        std::env::set_var("GIT_CONFIG_PARAMETERS", "'alias.status'='log'");
        std::env::set_var("GIT_CONFIG", "/nonexistent/config");
    }
    let _scratch = Scratch::new();
    for name in ["GIT_CONFIG_COUNT", "GIT_CONFIG_PARAMETERS", "GIT_CONFIG"] {
        assert_eq!(std::env::var_os(name), None, "{name} is still set");
    }
}

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

/// A byte copy of `repo`, in which the user's own git can be asked what it would have
/// done, without touching the real one.
fn byte_copy(scratch: &Scratch, repo: &Path, name: &str) -> PathBuf {
    let copy = scratch.join(name);
    let status = std::process::Command::new("cp")
        .arg("-R")
        .arg(repo)
        .arg(&copy)
        .status()
        .expect("copy");
    assert!(status.success(), "cp -R failed");
    copy
}

/// The pointer a clean filter standing in for Git LFS prints for a file of `size` bytes: a
/// fixed hash, and the size it read, so that a change of the file is a change of the
/// pointer.
fn pointer(size: usize) -> String {
    format!(
        "version https://git-lfs.github.com/spec/v1\noid sha256:\
         0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\nsize {size}\n"
    )
}

// STATE-02. The tree of a state is the one `git add -A` writes in a byte copy of the
// repository, with every kind of file whose blob git makes by more than copying the file.
#[test]
fn the_tree_of_a_state_is_the_tree_a_commit_of_everything_would_have_through_every_filter() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    // The stand-in for Git LFS: the CI machine has none. A POSIX shell script that reads
    // its input to the end and prints the pointer, as the real clean filter does.
    let lfs = scratch.join("bin/lfs-clean");
    program(
        &lfs,
        // The size is counted by reading the input to its end; `wc` of some systems pads
        // the number with spaces.
        "size=$(wc -c | tr -d ' ')\nprintf 'version https://git-lfs.github.com/spec/v1\\n\
         oid sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\\n\
         size %s\\n' \"$size\"",
    );
    let config = repo.join(".git/config");
    let mut text = std::fs::read_to_string(&config).expect("read the configuration");
    // Written into the file, as a user's own settings are.
    text.push_str(&format!(
        "[core]\n\tautocrlf = true\n\
         [filter \"upper\"]\n\tclean = tr a-z A-Z\n\
         [filter \"lfs\"]\n\tclean = '{lfs}'\n\tsmudge = cat\n\trequired = true\n",
        lfs = lfs.display()
    ));
    std::fs::write(&config, text).expect("write the configuration");
    write(&repo, ".gitignore", "*.log\n");
    write(
        &repo,
        ".gitattributes",
        "*.txt filter=upper\n*.bin filter=lfs\n",
    );
    write(&repo, "notes.txt", "one\n");
    write(&repo, "lines.md", "a\r\nb\r\n");
    write(&repo, "kept.log", "first\n");
    git(&repo, &["add", "-f", "kept.log"]);
    write(&repo, "model.bin", "12345");
    commit_all(&repo, "start");

    // Each kind, changed after the commit.
    write(&repo, "notes.txt", "two lines\nmore\n");
    write(&repo, "lines.md", "a\r\nb\r\nc\r\n");
    write(&repo, "kept.log", "second\n");
    // The large file the pointer stands in for, which is longer now, and a text file that
    // is new; and an ignored file that is force-added and staged, never committed.
    write(&repo, "model.bin", "1234567");
    write(&repo, "dir/new.txt", "new text\n");
    write(&repo, "staged.log", "forced in\n");
    git(&repo, &["add", "-f", "staged.log"]);
    write(&repo, "ignored.log", "stays out\n");
    let copy = byte_copy(&scratch, &repo, "copy");
    git(&copy, &["add", "-A"]);
    let wanted = git(&copy, &["write-tree"]);
    let store = store(&scratch);

    let state = store.record(&open(&repo)).expect("record");

    assert_eq!(state.tree, wanted);
    // What git wrote for each file in the copy, and what each must be by the rule that
    // makes it: the clean filter, the line ends, the forced file as it is now, the pointer.
    let blob = |path: &str| git_bytes(&copy, &["cat-file", "blob", &format!("{wanted}:{path}")]);
    for (path, content) in [
        ("notes.txt", "TWO LINES\nMORE\n"),
        ("dir/new.txt", "NEW TEXT\n"),
        ("lines.md", "a\nb\nc\n"),
        ("kept.log", "second\n"),
        ("staged.log", "forced in\n"),
        ("model.bin", &pointer(7)),
    ] {
        assert_eq!(blob(path), content.as_bytes(), "git's blob of {path}");
        assert_eq!(
            store
                .content(state.id.as_str(), Path::new(path))
                .expect("read"),
            Some(content.as_bytes().to_vec()),
            "the state's content of {path}"
        );
    }
    // An ignored file that nothing forced in is in no commit of everything.
    assert_eq!(
        store
            .content(state.id.as_str(), Path::new("ignored.log"))
            .expect("read"),
        None
    );
}

// What `git add -A` does with a path that is a file where it was a folder, and a folder
// where it was a file, which a build or a checkout of another branch does all the time.
#[test]
fn a_folder_that_became_a_file_and_a_file_that_became_a_folder_are_in_the_state_as_git_add_has_them()
 {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "swap/inner.txt", "inside\n");
    write(&repo, "flip.txt", "a file\n");
    write(&repo, "same.txt", "same\n");
    commit_all(&repo, "start");
    std::fs::remove_dir_all(repo.join("swap")).expect("remove the folder");
    write(&repo, "swap", "now a file\n");
    std::fs::remove_file(repo.join("flip.txt")).expect("remove the file");
    write(&repo, "flip.txt/in.txt", "now a folder\n");
    let copy = byte_copy(&scratch, &repo, "copy");
    git(&copy, &["add", "-A"]);
    let wanted = git(&copy, &["write-tree"]);
    let store = store(&scratch);

    let state = store.record(&open(&repo)).expect("record");

    assert_eq!(state.tree, wanted);
}

/// Where each place a hook can be in, one repository for each so that only that place is
/// the one git would read the hook from.
#[derive(Clone, Copy, Debug)]
enum HookPlace {
    /// In the hooks folder of the repository.
    Folder,
    /// In a folder that a global configuration names as `core.hooksPath`.
    Global,
    /// In configuration, as `hook.<name>.command` with its event.
    Configuration,
}

// STATE-03. Each place a hook can be in gets a repository of its own: a global hooks path
// replaces the hooks folder, so one repository could not have both at work.
#[test]
fn with_a_failing_hook_in_the_repository_behind_a_global_path_and_in_configuration_a_state_is_taken_and_none_runs()
 {
    let scratch = Scratch::new();
    let global = scratch.join("global-config");
    std::fs::write(
        &global,
        format!(
            "[core]\n\thooksPath = {}\n",
            scratch.join("global-hooks").display()
        ),
    )
    .expect("write the global configuration");
    let note = |place: HookPlace, hook: &str| {
        format!(
            "echo {hook} >> '{}'\nexit 1",
            scratch.join(&format!("ran-{place:?}")).display()
        )
    };

    let mut supported = Vec::new();
    for place in [
        HookPlace::Folder,
        HookPlace::Global,
        HookPlace::Configuration,
    ] {
        // Only the global place reads the global file: a global hooks path would
        // replace the hooks folder of every other repository of the test.
        // SAFETY: changing the environment is sound only while no other thread reads it:
        // this test starts none, and `cargo nextest` gives it a process of its own.
        unsafe {
            std::env::set_var(
                "GIT_CONFIG_GLOBAL",
                if matches!(place, HookPlace::Global) {
                    global.as_path()
                } else {
                    Path::new("/dev/null")
                },
            );
        }
        let repo = scratch.join(&format!("repo-{place:?}"));
        init(&repo);
        write(&repo, "a.txt", "one\n");
        commit_all(&repo, "start");
        for hook in ["post-index-change", "pre-commit"] {
            let script = note(place, hook);
            match place {
                HookPlace::Folder => program(&repo.join(".git/hooks").join(hook), &script),
                HookPlace::Global => program(&scratch.join("global-hooks").join(hook), &script),
                HookPlace::Configuration => {
                    let script_file = scratch.join(&format!("config-{hook}"));
                    program(&script_file, &script);
                    let config = repo.join(".git/config");
                    let mut text = std::fs::read_to_string(&config).expect("read");
                    // Both keys: with no event a hook is none.
                    text.push_str(&format!(
                        "[hook \"yaar-{hook}\"]\n\tcommand = {}\n\tevent = {hook}\n",
                        script_file.display()
                    ));
                    std::fs::write(&config, text).expect("write the configuration");
                }
            }
        }
        write(&repo, "a.txt", "changed\n");
        write(&repo, "b.txt", "new\n");

        // The premise, in a byte copy: the hook of this place refuses a commit, where this
        // git runs hooks of that place at all. A git that does not is told so.
        let copy = byte_copy(&scratch, &repo, &format!("copy-{place:?}"));
        let refused = std::process::Command::new("git")
            .arg("-C")
            .arg(&copy)
            .args(["commit", "--quiet", "-am", "x"])
            .stdin(std::process::Stdio::null())
            .output()
            .expect("start git");
        let ran = scratch.join(&format!("ran-{place:?}"));
        if refused.status.success() {
            assert!(
                matches!(place, HookPlace::Configuration),
                "git ran no hook of {place:?}, which every git does"
            );
            println!("hooks in configuration are not supported by this git: {place:?} skipped");
            assert!(
                !ran.exists(),
                "a hook ran in the copy, and no commit was refused"
            );
            continue;
        }
        assert!(
            ran.exists(),
            "the commit was refused by something else than the hook of {place:?}"
        );
        println!("hooks of {place:?} are run by this git");
        supported.push(place);
        std::fs::remove_file(&ran).expect("clear what the copy's hooks wrote");

        let store = StateStore::open(&found(), &scratch.join(&format!("store-{place:?}.git")))
            .expect("open the store");
        let state = store.record(&open(&repo)).expect("record");

        let held = held_by(&store, &state);
        assert_eq!(held.get("a.txt"), Some(&b"changed\n".to_vec()), "{place:?}");
        assert_eq!(held.get("b.txt"), Some(&b"new\n".to_vec()), "{place:?}");
        assert_eq!(
            store.states().expect("states"),
            vec![state],
            "{place:?}: the state is held"
        );
        assert!(
            !ran.exists(),
            "{place:?}: a hook ran: {:?}",
            std::fs::read_to_string(&ran)
        );
    }
    assert!(
        supported.len() >= 2,
        "the hooks folder and the global hooks path are run by every git: {supported:?}"
    );
}

// STATE-06. A program that creates and deletes files beside the recordings, and flips a
// path between a file and a folder, as a build does.
#[test]
fn forty_states_taken_beside_a_loop_that_creates_and_deletes_files_all_succeed() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    write(&repo, "churn/kept.txt", "kept\n");
    commit_all(&repo, "start");
    let worktree = open(&repo);
    let store = store(&scratch);
    let stop = std::sync::atomic::AtomicBool::new(false);
    let churn = repo.join("churn");

    // Tells the loop to end however the recordings end: a failed one panics, and the scope
    // waits for the loop.
    struct Stops<'a>(&'a std::sync::atomic::AtomicBool);
    impl Drop for Stops<'_> {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    let made = std::thread::scope(|scope| {
        let _stops = Stops(&stop);
        let loop_of_files = scope.spawn(|| {
            let mut made = 0_u64;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let n = made % 64;
                let file = churn.join(format!("f{n}"));
                std::fs::write(&file, format!("{made}\n")).expect("create");
                // Deleted a few files behind, so that some are there when git looks and
                // gone when it stages them.
                let _ = std::fs::remove_file(churn.join(format!("f{}", (n + 60) % 64)));
                if made.is_multiple_of(7) {
                    // A path that is a file, then a folder with a file in it, then a file.
                    let flip = churn.join("flip");
                    let _ = std::fs::remove_dir_all(&flip);
                    let _ = std::fs::write(&flip, "a file\n");
                    let _ = std::fs::remove_file(&flip);
                    let _ = std::fs::create_dir(&flip);
                    let _ = std::fs::write(flip.join("inside"), "in a folder\n");
                }
                made += 1;
            }
            made
        });
        let mut states = Vec::new();
        for n in 0..40 {
            let state = store
                .record(&worktree)
                .unwrap_or_else(|e| panic!("recording {n} of 40 failed: {e}"));
            // A tree git can read: its files list.
            store
                .paths(state.id.as_str())
                .unwrap_or_else(|e| panic!("the state {n} does not read: {e}"));
            states.push(state);
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        println!(
            "the loop made {} files",
            loop_of_files.join().expect("the loop")
        );
        states
    });

    assert_eq!(made.len(), 40);
    assert!(!store.states().expect("states").is_empty());
}

/// Configures the clean filter `name` of the repository as `program`, one that has to
/// succeed for git to stage a file.
fn required_filter(repo: &Path, name: &str, program: &Path) {
    use std::io::Write;
    let mut configuration = std::fs::OpenOptions::new()
        .append(true)
        .open(repo.join(".git/config"))
        .expect("open the configuration");
    write!(
        configuration,
        "[filter \"{name}\"]\n\tclean = '{}'\n\trequired = true\n",
        program.display()
    )
    .expect("configure the filter");
}

// A path git cannot stage for a reason of its own, as a clean filter that fails, fails the
// recording with git's words. It is found by cutting the list in halves, not by staging
// each path on its own in a run of git of its own, which for a worktree of tens of
// thousands of files would be that many runs at every recording.
#[test]
fn a_path_that_cannot_be_staged_fails_the_recording_after_a_few_runs_of_git_and_not_one_for_each_path()
 {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    let filter = scratch.join("bin/cannot-read");
    program(&filter, "echo 'the filter cannot read it' >&2\nexit 1");
    required_filter(&repo, "bad", &filter);
    write(&repo, ".gitattributes", "*.bad filter=bad\n");
    for n in 0..100 {
        write(&repo, &format!("f{n:03}.txt"), &format!("{n}\n"));
    }
    // The last of the paths, in the order git lists them.
    write(&repo, "zz.bad", "unreadable\n");
    let (program, list) = support::git_that_keeps_a_list(&scratch);
    let git = Git::at(program);
    let worktree = Worktree::open(&git, &repo).expect("open");
    let store = StateStore::open(&git, &scratch.join("app/states/one.git")).expect("open");

    let error = store.record(&worktree).expect_err("no state");

    assert!(
        error.to_string().contains("the filter cannot read it"),
        "git's words: {error}"
    );
    let staging_runs = std::fs::read_to_string(&list)
        .expect("the list of starts")
        .lines()
        .filter(|start| start.contains("update-index --add"))
        .count();
    assert!(
        (1..=20).contains(&staging_runs),
        "{staging_runs} runs of git to stage 102 paths"
    );
    assert!(store.states().expect("states").is_empty());
}

/// A repository in which `path`, a file of `.race`, is not committed, and a clean filter
/// for such files that runs `effect` on the worktree, as a program beside the recording
/// would, and then fails: git has named the path, and by the time it fails to stage it
/// the worktree is not as it was.
fn with_a_file_that_changes_as_it_is_staged(
    scratch: &Scratch,
    path: &str,
    effect: &str,
) -> PathBuf {
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, ".gitattributes", "*.race filter=race\n");
    write(&repo, "kept.txt", "kept\n");
    commit_all(&repo, "start");
    let filter = scratch.join("bin/disturb");
    program(&filter, &format!("{effect}\nexit 1"));
    required_filter(&repo, "race", &filter);
    write(&repo, path, "here\n");
    repo
}

// A path that is gone when git fails to stage it is no failure: it is what the worktree
// looks like by then.
#[test]
fn a_path_that_is_gone_when_it_fails_to_be_staged_is_left_out_and_the_state_is_taken() {
    let scratch = Scratch::new();
    let file = scratch.join("repo/gone.race");
    let repo = with_a_file_that_changes_as_it_is_staged(
        &scratch,
        "gone.race",
        &format!("rm -f '{}'", file.display()),
    );
    let store = store(&scratch);

    let state = store.record(&open(&repo)).expect("record");

    assert_eq!(
        store.paths(state.id.as_str()).expect("paths"),
        [PathBuf::from(".gitattributes"), PathBuf::from("kept.txt")]
    );
}

// The same where the folder above the path is a file by then, which is not a missing path
// to the system but is no path either.
#[test]
fn a_path_whose_folder_became_a_file_when_it_fails_to_be_staged_is_left_out_too() {
    let scratch = Scratch::new();
    let folder = scratch.join("repo/folder");
    let repo = with_a_file_that_changes_as_it_is_staged(
        &scratch,
        "folder/inside.race",
        &format!("rm -rf '{0}'\necho 'a file now' > '{0}'", folder.display()),
    );
    let store = store(&scratch);

    let state = store.record(&open(&repo)).expect("record");

    assert_eq!(
        store.paths(state.id.as_str()).expect("paths"),
        [PathBuf::from(".gitattributes"), PathBuf::from("kept.txt")]
    );
}

// Among many paths the one that changed is found without failing the others.
#[test]
fn a_path_that_is_gone_when_it_fails_to_be_staged_among_many_leaves_the_others_in_the_state() {
    let scratch = Scratch::new();
    let file = scratch.join("repo/gone.race");
    let repo = with_a_file_that_changes_as_it_is_staged(
        &scratch,
        "gone.race",
        &format!("rm -f '{}'", file.display()),
    );
    for n in 0..30 {
        write(&repo, &format!("f{n:02}.txt"), &format!("{n}\n"));
    }
    let store = store(&scratch);

    let state = store.record(&open(&repo)).expect("record");

    let paths = store.paths(state.id.as_str()).expect("paths");
    assert_eq!(paths.len(), 2 + 30, "{paths:?}");
    assert!(!paths.contains(&PathBuf::from("gone.race")));
}

// STATE-08, as `record` shows it: the recording that used to fail with git's words now
// succeeds.
#[test]
fn a_repository_inside_the_worktree_with_no_commit_is_left_out_of_the_state() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "start");
    write(&repo, "a.txt", "changed\n");
    init(&repo.join("inner"));
    write(&repo.join("inner"), "i.txt", "inside\n");
    let store = store(&scratch);

    let state = store
        .record(&open(&repo))
        .expect("a recording that succeeds");

    assert_eq!(
        store.paths(state.id.as_str()).expect("paths"),
        vec![PathBuf::from("a.txt")]
    );
    assert_eq!(store.states().expect("states"), vec![state]);
}

// STATE-08. A repository with a commit is a link to that commit, one with none holds
// nothing git could record: it is left out and named.
#[test]
fn an_untracked_repository_with_no_commit_is_left_out_of_the_state_and_reported() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "start");
    write(&repo, "a.txt", "changed\n");
    // Two with no commit, one of them far down and one whose git folder lies elsewhere,
    // so that `.git` in it is a file; and one with a commit.
    let nested = repo.join("nested");
    init(&nested);
    write(&nested, "n.txt", "inside\n");
    write(&nested, "deeper/n.txt", "further inside\n");
    let deep = repo.join("tools/vendor/deep");
    init(&deep);
    write(&deep, "d.txt", "down\n");
    let apart = repo.join("apart");
    git(
        &repo,
        &[
            "init",
            "--quiet",
            "-b",
            "main",
            &format!("--separate-git-dir={}", scratch.join("apart.git").display()),
            apart.to_str().unwrap(),
        ],
    );
    write(&apart, "x.txt", "elsewhere\n");
    assert!(apart.join(".git").is_file(), "the premise: .git is a file");
    let inner = repo.join("inner");
    init(&inner);
    write(&inner, "i.txt", "inside\n");
    let inner_head = commit_all(&inner, "inside");
    let store = store(&scratch);

    let recording = store
        .record_reporting(&open(&repo))
        .expect("a recording that succeeds");

    // The changed file is in, the repository with a commit is its commit, and nothing
    // lies under a repository with none.
    assert_eq!(
        store.paths(recording.state.id.as_str()).expect("paths"),
        vec![PathBuf::from("a.txt"), PathBuf::from("inner")]
    );
    assert_eq!(
        store
            .content(recording.state.id.as_str(), Path::new("a.txt"))
            .expect("read"),
        Some(b"changed\n".to_vec())
    );
    assert_eq!(
        git(store.folder(), &["ls-tree", &recording.state.tree, "inner"]),
        format!("160000 commit {inner_head}\tinner")
    );
    // Named by their paths, ordered, without a slash at the end.
    assert_eq!(
        recording.left_out,
        vec![
            PathBuf::from("apart"),
            PathBuf::from("nested"),
            PathBuf::from("tools/vendor/deep")
        ]
    );
    // Recording again with nothing changed is the same state, and says the same.
    let again = store
        .record_reporting(&open(&repo))
        .expect("a second recording");
    assert_eq!(again, recording);
    assert_eq!(store.states().expect("states").len(), 1);
    // A recording that leaves nothing out says so.
    let plain = scratch.join("plain");
    init(&plain);
    write(&plain, "a.txt", "one\n");
    let other = StateStore::open(&found(), &scratch.join("app/states/two.git")).expect("open");
    assert_eq!(
        other
            .record_reporting(&open(&plain))
            .expect("record")
            .left_out,
        Vec::<PathBuf>::new()
    );
}

// What is left out joins the state as soon as it has a commit.
#[test]
fn a_repository_left_out_joins_the_state_when_it_gets_its_first_commit() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    commit_all(&repo, "start");
    let nested = repo.join("nested");
    init(&nested);
    write(&nested, "n.txt", "inside\n");
    let store = store(&scratch);
    let worktree = open(&repo);
    let first = store.record_reporting(&worktree).expect("record");
    assert_eq!(first.left_out, vec![PathBuf::from("nested")]);

    let nested_head = commit_all(&nested, "first");
    let second = store.record_reporting(&worktree).expect("record again");

    assert_eq!(second.left_out, Vec::<PathBuf>::new());
    assert_ne!(second.state, first.state);
    assert_eq!(
        git(store.folder(), &["ls-tree", &second.state.tree, "nested"]),
        format!("160000 commit {nested_head}\tnested")
    );
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

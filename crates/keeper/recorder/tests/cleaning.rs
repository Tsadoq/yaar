//! Component tests: the time a state is recorded at, the deletion of the states and kept
//! commits a caller names, the packing that removes what nothing reaches, and the room the
//! stores take on disk, through the surface the rest of the Keeper uses, in real
//! repositories the tests make.

mod support;

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::os::unix::fs::{MetadataExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use support::{Scratch, commit_all, git, init, write};
use yaar_recorder::{
    Clock, CommitId, Deleted, Git, RecordedState, SAME_STATE_FOR, StateStore, Worktree, sizes,
};

/// 2026-01-01T00:00:00Z, which the clock of every test starts at.
const BASE: u64 = 1_767_225_600;

/// How long a test waits for work that must end, which a deadlock never does.
const DEADLINE: Duration = Duration::from_secs(300);

/// A clock that starts at [`BASE`] and moves only when the test moves it.
#[derive(Debug)]
struct TestClock {
    base: SystemTime,
    moved: Mutex<Duration>,
}

impl TestClock {
    fn new() -> Arc<TestClock> {
        Arc::new(TestClock {
            base: UNIX_EPOCH + Duration::from_secs(BASE),
            moved: Mutex::new(Duration::ZERO),
        })
    }

    fn move_by(&self, by: Duration) {
        *self.moved.lock().expect("the clock") += by;
    }
}

impl Clock for TestClock {
    fn now(&self) -> SystemTime {
        self.base + *self.moved.lock().expect("the clock")
    }
}

/// The seconds `time` lies after the epoch.
fn seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_secs()
}

fn found() -> Git {
    Git::find().expect("git on the search path")
}

fn open(folder: &Path) -> Worktree {
    Worktree::open(&found(), folder).expect("open")
}

fn open_store(folder: &Path, clock: &Arc<TestClock>) -> StateStore {
    StateStore::open_with(&found(), folder, clock.clone()).expect("open the store")
}

/// Pseudo-random bytes from a xorshift of a seed: incompressible, and none of two seeds
/// share a run.
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn kib(&mut self, kib: usize) -> Vec<u8> {
        (0..kib * 128)
            .flat_map(|_| self.next().to_le_bytes())
            .collect()
    }
}

/// Writes `turn.bin` anew with 64 KiB nothing else holds, and records the worktree.
fn record_with_noise(
    store: &StateStore,
    repo: &Path,
    worktree: &Worktree,
    noise: &mut Noise,
) -> RecordedState {
    std::fs::write(repo.join("turn.bin"), noise.kib(64)).expect("write turn.bin");
    store.record(worktree).expect("record")
}

/// A repository with a tracked text file and an untracked `turn.bin`, one commit.
fn repository(scratch: &Scratch, name: &str) -> PathBuf {
    let repo = scratch.join(name);
    init(&repo);
    write(&repo, "notes.txt", "one\ntwo\nthree\n");
    commit_all(&repo, "one");
    std::fs::write(repo.join("turn.bin"), b"first").expect("write turn.bin");
    repo
}

/// Puts two objects in the repository that no ref reaches: a blob, and a commit that
/// `git stash create` made and left unnamed. The tracked file is as it was after.
fn plant(scratch: &Scratch, repo: &Path, name: &str) -> [String; 2] {
    let file = scratch.join(&format!("planted-{name}.txt"));
    std::fs::write(&file, format!("a blob nothing names in {name}\n")).expect("write the file");
    let blob = git(repo, &["hash-object", "-w", file.to_str().expect("text")]);
    write(repo, "notes.txt", "edited\n");
    let commit = git(repo, &["stash", "create"]);
    git(repo, &["checkout", "--", "notes.txt"]);
    assert!(!commit.is_empty(), "the stash was created");
    [blob, commit]
}

/// What a read-only look at a repository gives, by the seven reads of the plan.
#[derive(Debug, PartialEq, Eq)]
struct Reads {
    status: Vec<u8>,
    refs: Vec<u8>,
    worktrees: Vec<u8>,
    stash: Vec<u8>,
    unreachable: Vec<u8>,
    index: Vec<u8>,
    config: Vec<u8>,
    objects: Vec<(String, u64)>,
}

fn reads(repo: &Path) -> Reads {
    let run = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .args(args)
            .output()
            .expect("start git");
        assert!(output.status.success(), "git {args:?} failed");
        output.stdout
    };
    let common = git(repo, &["rev-parse", "--git-common-dir"]);
    let common = repo.join(common);
    Reads {
        status: run(&[
            "status",
            "--porcelain=v2",
            "--ignored",
            "--untracked-files=all",
        ]),
        refs: run(&["for-each-ref"]),
        worktrees: run(&["worktree", "list", "--porcelain"]),
        stash: run(&["stash", "list"]),
        unreachable: run(&["fsck", "--unreachable", "--no-reflogs", "--no-progress"]),
        index: std::fs::read(repo.join(".git/index")).expect("the list of files"),
        config: std::fs::read(repo.join(".git/config")).expect("the configuration"),
        objects: files_below(&common.join("objects")),
    }
}

/// Every file below `folder` with its size, by its path relative to `folder`, in order.
fn files_below(folder: &Path) -> Vec<(String, u64)> {
    fn walk(root: &Path, folder: &Path, into: &mut Vec<(String, u64)>) {
        for child in std::fs::read_dir(folder).expect("read the folder") {
            let child = child.expect("an entry").path();
            let kind = std::fs::symlink_metadata(&child).expect("the kind");
            if kind.is_dir() {
                walk(root, &child, into);
            } else {
                let name = child.strip_prefix(root).expect("below the root");
                into.push((name.to_string_lossy().into_owned(), kind.len()));
            }
        }
    }
    let mut found = Vec::new();
    walk(folder, folder, &mut found);
    found.sort();
    found
}

/// A repository as `reads` and `plant` leave it: the two planted objects are still there.
fn assert_unchanged(repo: &Path, before: &Reads, planted: &[String; 2]) {
    assert!(
        &reads(repo) == before,
        "the repository's reads changed: {repo:?}"
    );
    for id in planted {
        git(repo, &["cat-file", "-e", id]);
    }
}

/// What the files under `path` take on disk, as the test counts it: the blocks of every
/// entry, `path` itself included, links not followed.
fn blocks_of(path: &Path) -> u64 {
    let meta = std::fs::symlink_metadata(path).expect("the entry");
    let mut total = meta.blocks() * 512;
    if meta.is_dir() {
        for child in std::fs::read_dir(path).expect("read the folder") {
            total += blocks_of(&child.expect("an entry").path());
        }
    }
    total
}

/// What `du -sk` says `path` takes, in bytes.
fn du_bytes(path: &Path) -> u64 {
    let output = Command::new("du")
        .arg("-sk")
        .arg(path)
        .output()
        .expect("start du");
    assert!(output.status.success(), "du failed");
    let text = String::from_utf8(output.stdout).expect("text");
    let kib: u64 = text
        .split_whitespace()
        .next()
        .expect("a number")
        .parse()
        .expect("a number");
    kib * 1024
}

/// The loose objects of a store, by id.
fn loose_objects(store: &Path) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for fan in std::fs::read_dir(store.join("objects")).expect("read the objects") {
        let fan = fan.expect("an entry").path();
        let name = fan
            .file_name()
            .expect("a name")
            .to_string_lossy()
            .into_owned();
        if name.len() == 2 && name.bytes().all(|b| b.is_ascii_hexdigit()) {
            for object in std::fs::read_dir(&fan).expect("read the folder") {
                let object = object.expect("an entry").file_name();
                found.insert(format!("{name}{}", object.to_string_lossy()));
            }
        }
    }
    found
}

/// Whether the store holds the object.
fn holds(store: &Path, id: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(store)
        .args(["cat-file", "-e", id])
        .status()
        .expect("start git")
        .success()
}

/// The names of the refs of the store with what they name, as git lists them.
fn refs_of(store: &Path) -> String {
    git(store, &["for-each-ref"])
}

/// Every file of a state with its content, by its path.
fn files_of(store: &StateStore, state: &RecordedState) -> Vec<(PathBuf, Vec<u8>)> {
    store
        .paths(state.id.as_str())
        .expect("list")
        .into_iter()
        .map(|path| {
            let content = store
                .content(state.id.as_str(), &path)
                .expect("read")
                .unwrap_or_else(|| panic!("the state has no content for {path:?}"));
            (path, content)
        })
        .collect()
}

/// Runs `work` on a thread of its own and waits for its end for [`DEADLINE`]: a deadlock
/// is a failure that says so, not a test that never ends.
fn ends_in_time(what: &str, work: impl FnOnce() + Send + 'static) {
    let (done, ended) = channel();
    std::thread::spawn(move || {
        work();
        let _ = done.send(());
    });
    match ended.recv_timeout(DEADLINE) {
        Ok(()) => {}
        Err(RecvTimeoutError::Timeout) => panic!("{what} did not end: a deadlock"),
        Err(RecvTimeoutError::Disconnected) => panic!("{what} failed in a thread"),
    }
}

#[test]
fn the_sizes_of_the_recorded_states_are_what_they_take_on_disk_and_the_clean_up_removes_what_nothing_refers_to()
 {
    let scratch = Scratch::new();
    let repo_a = repository(&scratch, "repo-a");
    let repo_b = repository(&scratch, "repo-b");
    let planted_a = plant(&scratch, &repo_a, "a");
    let planted_b = plant(&scratch, &repo_b, "b");
    let reads_a = reads(&repo_a);
    let reads_b = reads(&repo_b);
    let clock = TestClock::new();
    let states = scratch.join("states");
    let (worktree_a, worktree_b) = (open(&repo_a), open(&repo_b));
    let mut noise = Noise(0x9E37_79B9_7F4A_7C15);

    // Three states in a, one in b, each holding 64 KiB no other holds.
    let before_cleaning = {
        let a = open_store(&states.join("a.git"), &clock);
        let b = open_store(&states.join("b.git"), &clock);
        let [a1, a2, a3] =
            [(); 3].map(|()| record_with_noise(&a, &repo_a, &worktree_a, &mut noise));
        record_with_noise(&b, &repo_b, &worktree_b, &mut noise);
        // A comparison of the files on disk with the first state writes a tree and a blob
        // that no ref names.
        std::fs::write(repo_a.join("turn.bin"), noise.kib(64)).expect("write turn.bin");
        let loose_before = loose_objects(a.folder());
        let changed = a
            .compare(Some(&worktree_a), a1.id.as_str(), None)
            .expect("compare");
        assert_eq!(changed.len(), 1, "only turn.bin differs from a1 on disk");
        let unnamed: Vec<String> = loose_objects(a.folder())
            .difference(&loose_before)
            .cloned()
            .collect();
        assert!(
            unnamed.len() >= 2,
            "the comparison wrote a tree and a blob: {unnamed:?}"
        );
        let blob_of = |state: &RecordedState| {
            git(
                a.folder(),
                &["rev-parse", &format!("{}:turn.bin", state.id)],
            )
        };
        let (blobs_1_3, blob_2) = ([blob_of(&a1), blob_of(&a3)], blob_of(&a2));
        (a1, a2, a3, blobs_1_3, blob_2, unnamed)
        // Both stores are dropped here: what the sizes read is on disk.
    };
    let (a1, a2, a3, blobs_1_3, blob_2, unnamed) = before_cleaning;

    // "The disk space reported ... for each workspace and in total, is what they take on
    // disk": the test's own walk of the folders, and `du`, a second count by a program.
    let reported = sizes(&states).expect("sizes");
    let names: Vec<OsString> = reported
        .stores
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    assert_eq!(names, ["a.git", "b.git"]);
    let (size_a, size_b) = (reported.stores[0].1, reported.stores[1].1);
    assert_eq!(size_a, blocks_of(&states.join("a.git")));
    assert_eq!(size_b, blocks_of(&states.join("b.git")));
    assert_eq!(reported.total, blocks_of(&states));
    for (counted, folder) in [
        (size_a, states.join("a.git")),
        (size_b, states.join("b.git")),
        (reported.total, states.clone()),
    ] {
        let by_du = du_bytes(&folder);
        assert!(
            counted.abs_diff(by_du) <= 1024,
            "{folder:?}: {counted} bytes counted, du says {by_du}"
        );
    }
    assert!(
        size_a >= 3 * 64 * 1024 && size_b >= 64 * 1024,
        "each state holds 64 KiB of its own: {size_a}, {size_b}"
    );

    // "The clean-up removes at once what nothing refers to."
    let a = open_store(&states.join("a.git"), &clock);
    let kept_files = files_of(&a, &a2);
    assert!(
        kept_files
            .iter()
            .any(|(path, content)| path == Path::new("turn.bin") && content.len() == 64 * 1024),
        "the second state holds its 64 KiB"
    );
    let ids: BTreeSet<String> = [&a1, &a3].map(|state| state.id.as_str().to_owned()).into();
    let deleted = a.delete(&ids).expect("delete");
    assert_eq!(
        deleted,
        Deleted {
            states: vec![a1.id.to_string(), a3.id.to_string()],
            commits: Vec::new(),
        }
    );
    a.pack().expect("pack");

    // At once, with no wait.
    assert_eq!(a.states().expect("states"), vec![a2.clone()]);
    assert_eq!(
        files_of(&a, &a2),
        kept_files,
        "the state that stays reads as before"
    );
    let gone: Vec<&str> = [a1.id.as_str(), a3.id.as_str()]
        .into_iter()
        .chain(blobs_1_3.iter().map(String::as_str))
        .chain(unnamed.iter().map(String::as_str))
        .collect();
    for id in gone {
        assert!(!holds(a.folder(), id), "{id} is still in the store");
    }
    assert!(
        holds(a.folder(), &blob_2),
        "the state that stays keeps its blob"
    );
    let after = sizes(&states).expect("sizes");
    assert!(
        after.stores[0].1 + 96 * 1024 <= size_a,
        "a.git went from {size_a} to {}",
        after.stores[0].1
    );
    assert_eq!(after.stores[1].1, size_b, "b.git is as it was");
    assert_eq!(
        after.total,
        blocks_of(&states),
        "the total is the folder's after the clean-up too"
    );

    drop(a);
    assert_unchanged(&repo_a, &reads_a, &planted_a);
    assert_unchanged(&repo_b, &reads_b, &planted_b);
}

#[test]
fn a_reader_of_a_state_that_stays_reads_the_same_bytes_after_the_clean_up() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, "repo");
    let worktree = open(&repo);
    let clock = TestClock::new();
    let store = open_store(&scratch.join("states/one.git"), &clock);
    let mut noise = Noise(7);
    let first = record_with_noise(&store, &repo, &worktree, &mut noise);
    let second = record_with_noise(&store, &repo, &worktree, &mut noise);
    // The one process that hands out contents is started, and has read.
    let path = Path::new("turn.bin");
    let before = store
        .content(second.id.as_str(), path)
        .expect("read")
        .expect("a file");

    store
        .delete(&[first.id.to_string()].into())
        .expect("delete");
    store.pack().expect("pack");

    let after = store
        .content(second.id.as_str(), path)
        .expect("read")
        .expect("a file");
    assert_eq!(after, before);
    assert_eq!(
        store.content(first.id.as_str(), path).expect("read"),
        None,
        "a state that was deleted holds no file"
    );
}

#[test]
fn a_state_recorded_after_the_caller_listed_the_states_is_not_deleted() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, "repo");
    let worktree = open(&repo);
    let clock = TestClock::new();
    let store = open_store(&scratch.join("states/one.git"), &clock);
    let mut noise = Noise(11);
    for _ in 0..2 {
        record_with_noise(&store, &repo, &worktree, &mut noise);
    }
    let listed: BTreeSet<String> = store
        .states()
        .expect("states")
        .iter()
        .map(|state| state.id.to_string())
        .collect();
    let late = record_with_noise(&store, &repo, &worktree, &mut noise);

    let deleted = store.delete(&listed).expect("delete");
    store.pack().expect("pack");

    assert_eq!(deleted.states.len(), 2);
    assert_eq!(store.states().expect("states"), vec![late.clone()]);
    assert_eq!(
        files_of(&store, &late).len(),
        2,
        "the late state reads whole"
    );
}

#[test]
fn a_state_is_dated_by_the_clock_its_store_was_opened_with() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, "repo");
    let worktree = open(&repo);
    let clock = TestClock::new();
    let store = open_store(&scratch.join("states/one.git"), &clock);
    let mut noise = Noise(3);

    let first = record_with_noise(&store, &repo, &worktree, &mut noise);
    clock.move_by(Duration::from_secs(90));
    let second = record_with_noise(&store, &repo, &worktree, &mut noise);
    clock.move_by(Duration::from_secs(31 * 24 * 3600));
    let third = record_with_noise(&store, &repo, &worktree, &mut noise);

    let at = |offset: u64| UNIX_EPOCH + Duration::from_secs(BASE + offset);
    assert_eq!(
        store.recorded_times().expect("times"),
        vec![
            (first.id.clone(), at(0)),
            (second.id.clone(), at(90)),
            (third.id.clone(), at(90 + 31 * 24 * 3600)),
        ],
        "each state with the clock's second, the oldest first"
    );
    // Author and committer are the same time: git's own tools show the one date.
    let both = git(
        store.folder(),
        &["log", "-1", "--format=%at %ct", third.id.as_str()],
    );
    let expected = BASE + 90 + 31 * 24 * 3600;
    assert_eq!(both, format!("{expected} {expected}"));
}

#[test]
fn a_store_opened_without_a_clock_dates_a_state_by_the_machine() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, "repo");
    let worktree = open(&repo);
    let store = StateStore::open(&found(), &scratch.join("states/one.git")).expect("open");

    let before = seconds(SystemTime::now());
    let state = store.record(&worktree).expect("record");
    let after = seconds(SystemTime::now());

    let times = store.recorded_times().expect("times");
    assert_eq!(times.len(), 1);
    assert_eq!(times[0].0, state.id);
    let recorded = seconds(times[0].1);
    assert!(
        (before..=after).contains(&recorded),
        "recorded at {recorded}, between {before} and {after}"
    );
}

#[test]
fn files_recorded_again_within_the_hour_are_the_newest_state_and_past_it_a_new_one() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, "repo");
    let worktree = open(&repo);
    let clock = TestClock::new();
    let store = open_store(&scratch.join("states/one.git"), &clock);
    let first = store.record(&worktree).expect("record");

    clock.move_by(Duration::from_secs(59 * 60));
    let within = store.record(&worktree).expect("record");
    assert_eq!(
        within, first,
        "59 minutes later the files are the newest state"
    );
    clock.move_by(Duration::from_secs(59));
    assert_eq!(
        store.record(&worktree).expect("record"),
        first,
        "one second short of the hour"
    );

    clock.move_by(Duration::from_secs(1));
    assert_eq!(SAME_STATE_FOR, Duration::from_secs(3600));
    let past = store.record(&worktree).expect("record");
    assert_ne!(
        past.id, first.id,
        "at the hour the same files are a new state"
    );
    assert_eq!(past.tree, first.tree);
    assert_eq!(past.on, first.on);
    let refs = refs_of(store.folder());
    assert!(
        refs.contains("refs/yaar/states/0000000001")
            && refs.contains("refs/yaar/states/0000000002"),
        "the new state has the next number: {refs}"
    );
    assert_eq!(
        store.recorded_times().expect("times"),
        vec![
            (first.id.clone(), UNIX_EPOCH + Duration::from_secs(BASE)),
            (
                past.id.clone(),
                UNIX_EPOCH + Duration::from_secs(BASE + 3600)
            ),
        ]
    );

    // Counted from the newest state: an hour after the new one the files are new again.
    clock.move_by(Duration::from_secs(61 * 60));
    let again = store.record(&worktree).expect("record");
    assert_ne!(again.id, past.id);
    assert_eq!(store.states().expect("states").len(), 3);
}

#[test]
fn deleting_an_empty_list_or_an_id_no_ref_names_deletes_nothing() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, "repo");
    let worktree = open(&repo);
    let clock = TestClock::new();
    let store = open_store(&scratch.join("states/one.git"), &clock);
    let state = store.record(&worktree).expect("record");
    let head = CommitId::new(git(&repo, &["rev-parse", "HEAD"]));
    store.keep_commit(&worktree, &head).expect("keep");
    let refs = refs_of(store.folder());
    assert_eq!(refs.lines().count(), 2, "a state and a kept commit: {refs}");

    assert_eq!(
        store.delete(&BTreeSet::new()).expect("delete"),
        Deleted::default()
    );
    let unknown: BTreeSet<String> = ["1".repeat(40), "abc".to_owned(), state.tree.clone()].into();
    assert_eq!(store.delete(&unknown).expect("delete"), Deleted::default());

    assert_eq!(refs_of(store.folder()), refs, "every ref is there");
    assert_eq!(store.states().expect("states"), vec![state]);
    assert_eq!(store.kept_commits().expect("kept"), vec![head.to_string()]);
}

#[test]
fn a_delete_takes_away_only_refs_of_states_and_kept_commits() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, "repo");
    let worktree = open(&repo);
    let clock = TestClock::new();
    let store = open_store(&scratch.join("states/one.git"), &clock);
    let state = store.record(&worktree).expect("record");
    // A ref of somebody else's in the store's own folder, naming the same object.
    git(
        store.folder(),
        &["update-ref", "refs/heads/not-ours", state.id.as_str()],
    );

    let deleted = store
        .delete(&[state.id.to_string()].into())
        .expect("delete");

    assert_eq!(deleted.states, vec![state.id.to_string()]);
    assert_eq!(
        refs_of(store.folder()),
        format!("{} commit\trefs/heads/not-ours", state.id)
    );
}

#[test]
fn a_kept_commit_in_the_list_is_let_go_and_after_the_pack_its_files_are_gone() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, "repo");
    let mut noise = Noise(5);
    std::fs::write(repo.join("unique.bin"), noise.kib(16)).expect("write");
    let commit = CommitId::new(commit_all(&repo, "two"));
    write(&repo, "notes.txt", "changed\n");
    commit_all(&repo, "three");
    let worktree = open(&repo);
    let clock = TestClock::new();
    let store = open_store(&scratch.join("states/one.git"), &clock);
    let state = store.record(&worktree).expect("record");
    store.keep_commit(&worktree, &commit).expect("keep");
    assert_eq!(
        store.kept_commits().expect("kept"),
        vec![commit.to_string()]
    );
    assert!(
        store
            .paths(commit.as_str())
            .expect("paths")
            .contains(&PathBuf::from("unique.bin"))
    );
    let blob = git(
        store.folder(),
        &["rev-parse", &format!("{commit}:unique.bin")],
    );

    let deleted = store
        .delete(&[commit.to_string(), state.id.to_string()].into())
        .expect("delete");
    store.pack().expect("pack");

    assert_eq!(
        deleted,
        Deleted {
            states: vec![state.id.to_string()],
            commits: vec![commit.to_string()],
        }
    );
    assert_eq!(store.kept_commits().expect("kept"), Vec::<String>::new());
    assert!(store.paths(commit.as_str()).is_err(), "its files are gone");
    assert!(!holds(store.folder(), &blob), "and its blob");
}

#[test]
fn packing_a_store_with_nothing_in_it_or_nothing_deleted_changes_no_state() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, "repo");
    let worktree = open(&repo);
    let clock = TestClock::new();
    let store = open_store(&scratch.join("states/one.git"), &clock);
    store.pack().expect("pack an empty store");

    let mut noise = Noise(13);
    let state = record_with_noise(&store, &repo, &worktree, &mut noise);
    let files = files_of(&store, &state);
    store.pack().expect("pack");
    store.pack().expect("pack again");

    assert_eq!(store.states().expect("states"), vec![state.clone()]);
    assert_eq!(files_of(&store, &state), files);
}

#[test]
fn the_pack_removes_the_complete_head_when_its_commit_went_and_the_next_recording_copies_it_whole()
{
    let scratch = Scratch::new();
    let repo = repository(&scratch, "repo");
    let mut noise = Noise(17);
    std::fs::write(repo.join("committed.bin"), noise.kib(16)).expect("write");
    commit_all(&repo, "two");
    let worktree = open(&repo);
    let clock = TestClock::new();
    let store = open_store(&scratch.join("states/one.git"), &clock);
    let marker = store.folder().join("yaar-complete-head");
    let state = record_with_noise(&store, &repo, &worktree, &mut noise);
    assert!(
        marker.exists(),
        "the recording noted the commit it copied whole"
    );

    store
        .delete(&[state.id.to_string()].into())
        .expect("delete");
    store.pack().expect("pack");

    assert!(!marker.exists(), "its commit went, and so does the note");
    let next = record_with_noise(&store, &repo, &worktree, &mut noise);
    let files = files_of(&store, &next);
    let names: Vec<&str> = files
        .iter()
        .map(|(path, _)| path.to_str().expect("text"))
        .collect();
    assert_eq!(names, ["committed.bin", "notes.txt", "turn.bin"]);
    assert_eq!(
        files[0].1,
        std::fs::read(repo.join("committed.bin")).expect("read"),
        "a file of the commit, which no state of the new recording had to copy"
    );
}

#[test]
fn the_pack_keeps_the_complete_head_while_a_state_still_reaches_its_commit() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, "repo");
    let worktree = open(&repo);
    let clock = TestClock::new();
    let store = open_store(&scratch.join("states/one.git"), &clock);
    let mut noise = Noise(19);
    let first = record_with_noise(&store, &repo, &worktree, &mut noise);
    let second = record_with_noise(&store, &repo, &worktree, &mut noise);
    let marker = store.folder().join("yaar-complete-head");
    let noted = std::fs::read_to_string(&marker).expect("the note");

    store
        .delete(&[second.id.to_string()].into())
        .expect("delete");
    store.pack().expect("pack");

    assert_eq!(std::fs::read_to_string(&marker).expect("the note"), noted);
    assert_eq!(store.states().expect("states"), vec![first]);
    let next = record_with_noise(&store, &repo, &worktree, &mut noise);
    assert_eq!(files_of(&store, &next).len(), 2);
}

#[test]
fn the_commit_the_changes_are_counted_from_is_read_by_a_store_opened_anew() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, "repo");
    let commit = CommitId::new(git(&repo, &["rev-parse", "HEAD"]));
    let clock = TestClock::new();
    let folder = scratch.join("states/one.git");
    {
        let store = open_store(&folder, &clock);
        assert_eq!(store.counted_from().expect("read"), None);
        store.note_counted_from(&commit).expect("note");
    }

    let again = open_store(&folder, &clock);

    assert_eq!(again.counted_from().expect("read"), Some(commit));
}

#[test]
fn a_folder_that_is_not_there_takes_nothing_and_holds_no_store() {
    let scratch = Scratch::new();

    let reported = sizes(&scratch.join("no-such-folder")).expect("sizes");

    assert_eq!(reported.total, 0);
    assert!(reported.stores.is_empty());
}

#[test]
fn a_store_that_is_a_link_is_listed_by_its_name_with_the_links_own_blocks() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, "repo");
    let worktree = open(&repo);
    let clock = TestClock::new();
    let states = scratch.join("states");
    let mut noise = Noise(23);
    {
        let store = open_store(&states.join("real.git"), &clock);
        record_with_noise(&store, &repo, &worktree, &mut noise);
    }
    // A store the user moved to another disk, which the folder names by a link, and a
    // file of nobody's among the stores.
    let outside = scratch.join("outside");
    std::fs::create_dir_all(&outside).expect("make the folder");
    std::fs::write(outside.join("big.bin"), noise.kib(256)).expect("write");
    symlink(&outside, states.join("moved.git")).expect("make a link");
    std::fs::write(states.join("stray.txt"), vec![1u8; 10_000]).expect("write");

    let reported = sizes(&states).expect("sizes");

    let link_blocks = std::fs::symlink_metadata(states.join("moved.git"))
        .expect("the link")
        .blocks()
        * 512;
    assert_eq!(
        reported.stores,
        vec![
            (OsString::from("moved.git"), link_blocks),
            (
                OsString::from("real.git"),
                blocks_of(&states.join("real.git"))
            ),
        ]
    );
    assert_eq!(
        reported.total,
        blocks_of(&states),
        "the total counts the link and the stray file, and nothing of the link's target"
    );
    assert!(
        reported.total < blocks_of(&outside) + blocks_of(&states.join("real.git")),
        "the target's 256 KiB are not in it"
    );
}

/// A repository with two commits of different files, a state of the files now, and the
/// store: what a comparison against a commit of the repository needs.
struct Stressed {
    repo: PathBuf,
    worktree: Arc<Worktree>,
    store: Arc<StateStore>,
    state: RecordedState,
    old_commit: String,
}

fn stressed(scratch: &Scratch) -> Stressed {
    let repo = repository(scratch, "repo");
    let mut noise = Noise(29);
    std::fs::write(repo.join("old.bin"), noise.kib(8)).expect("write");
    let old_commit = commit_all(&repo, "two");
    write(&repo, "notes.txt", "one\nthree\n");
    std::fs::remove_file(repo.join("old.bin")).expect("remove");
    commit_all(&repo, "three");
    let worktree = Arc::new(open(&repo));
    let clock = TestClock::new();
    let store = Arc::new(open_store(&scratch.join("states/one.git"), &clock));
    let state = store.record(&worktree).expect("record");
    Stressed {
        repo,
        worktree,
        store,
        state,
        old_commit,
    }
}

/// Deletes the kept commit and packs, `rounds` times, as the clean-up at once does.
fn clean_up_repeatedly(store: &StateStore, commit: &str, rounds: usize) {
    let ids: BTreeSet<String> = [commit.to_owned()].into();
    for _ in 0..rounds {
        store.delete(&ids).expect("delete");
        store.pack().expect("pack");
    }
}

#[test]
fn a_comparison_never_fails_while_the_store_is_cleaned_beside_it() {
    let scratch = Scratch::new();
    let Stressed {
        worktree,
        store,
        state,
        old_commit,
        repo,
    } = stressed(&scratch);
    let reads_before = reads(&repo);
    let against_commit = store
        .compare(Some(&worktree), &old_commit, Some(state.id.as_str()))
        .expect("compare");
    assert!(!against_commit.is_empty());
    // The files on disk, changed since the state: a comparison with them writes a tree
    // and a blob that no ref names, which a pack would remove if it could fall between
    // their writing and their reading.
    std::fs::write(repo.join("turn.bin"), Noise(31).kib(4)).expect("write turn.bin");
    let against_disk = store
        .compare(Some(&worktree), state.id.as_str(), None)
        .expect("compare");
    assert_eq!(against_disk.len(), 1);

    ends_in_time("the comparisons and the clean-ups", move || {
        let cleaner = {
            let (store, commit) = (store.clone(), old_commit.clone());
            std::thread::spawn(move || clean_up_repeatedly(&store, &commit, 50))
        };
        // Two threads compare while it runs: the first against a commit of the repository
        // that the store copies and the clean-up lets go, the second against the files on
        // disk.
        let with_commit = {
            let (store, worktree) = (store.clone(), worktree.clone());
            let (commit, state) = (old_commit.clone(), state.id.to_string());
            std::thread::spawn(move || {
                for compared in 0..200 {
                    let answer = store
                        .compare(Some(&worktree), &commit, Some(&state))
                        .unwrap_or_else(|e| panic!("comparison {compared} failed: {e}"));
                    assert_eq!(answer, against_commit, "comparison {compared}");
                }
            })
        };
        for compared in 0..200 {
            let answer = store
                .compare(Some(&worktree), state.id.as_str(), None)
                .unwrap_or_else(|e| panic!("comparison {compared} with the disk failed: {e}"));
            assert_eq!(answer, against_disk, "comparison {compared} with the disk");
        }
        with_commit.join().expect("the comparisons");
        cleaner.join().expect("the clean-ups");
    });
    assert_eq!(reads(&repo).refs, reads_before.refs);
}

#[test]
fn every_call_that_holds_a_version_works_beside_the_clean_up_and_none_waits_forever() {
    let scratch = Scratch::new();
    let Stressed {
        worktree,
        store,
        state,
        old_commit,
        ..
    } = stressed(&scratch);
    let commit = CommitId::new(old_commit.clone());
    let out = scratch.join("exports");
    std::fs::create_dir_all(&out).expect("make the folder");
    let notes = Path::new("notes.txt");
    let old_notes = store
        .content_in(Some(&worktree), &old_commit, notes)
        .expect("content")
        .expect("a file");

    ends_in_time("the calls and the clean-ups", move || {
        let cleaner = {
            let (store, commit) = (store.clone(), old_commit.clone());
            std::thread::spawn(move || clean_up_repeatedly(&store, &commit, 25))
        };
        for round in 0..40 {
            assert_eq!(
                store
                    .content_in(Some(&worktree), &old_commit, notes)
                    .expect("content_in"),
                Some(old_notes.clone()),
                "round {round}"
            );
            let into = out.join(round.to_string());
            store
                .export(Some(&worktree), &old_commit, &into)
                .unwrap_or_else(|e| panic!("export {round} failed: {e}"));
            assert!(into.join("old.bin").exists(), "round {round}");
            store.keep_commit(&worktree, &commit).expect("keep_commit");
            store.paths(state.id.as_str()).expect("paths of the state");
            store
                .changed_paths(state.id.as_str(), state.id.as_str())
                .expect("changed_paths");
        }
        cleaner.join().expect("the clean-ups");
    });
}

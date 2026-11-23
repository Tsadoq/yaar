//! Component tests: the time a state is recorded at, the deletion of the states and kept
//! commits a caller names, the packing that removes what nothing reaches, and the room the
//! stores take on disk, through the surface the rest of the Keeper uses, in real
//! repositories the tests make.

mod support;

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use support::{Scratch, commit_all, git, git_that_keeps_a_list, init, is_alive, program, write};
use yaar_recorder::{
    Change, ChangedPath, Clock, CommitId, Deleted, Git, RecordedState, SAME_STATE_FOR, StateStore,
    Worktree, sizes,
};

/// 2026-01-01T00:00:00Z, which the clock of every test starts at.
const BASE: u64 = 1_767_225_600;

/// How long a test waits for work that must end, which a deadlock never does.
const DEADLINE: Duration = Duration::from_secs(120);

/// How long a loop of calls may go on with none of its steps ending, which a deadlock never
/// does. How long the whole loop takes is the machine's to say, so only a stop is bounded.
const STEP_DEADLINE: Duration = Duration::from_secs(120);

/// How long a clean-up that ought to be waiting is given to show that it is not. Nothing a
/// passing run does has to end within it: a clean-up that runs on in that time is a failure,
/// one that is slower than this on a busy machine is missed, never wrongly blamed.
const SETTLE: Duration = Duration::from_secs(3);

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

/// A store whose every git is started through `program`.
fn open_store_through(program: &Path, folder: &Path, clock: &Arc<TestClock>) -> StateStore {
    StateStore::open_with(&Git::at(program), folder, clock.clone()).expect("open the store")
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

/// A repository with the two objects planted in it and its reads taken, before the
/// program under test is started: what a clean-up must leave as it found it.
struct Watched {
    repo: PathBuf,
    planted: [String; 2],
    before: Reads,
}

/// Plants the two objects in `repo` and takes its reads. Called after the test's own
/// commits and before anything of the Recorder runs.
fn watch(scratch: &Scratch, repo: &Path) -> Watched {
    let planted = plant(scratch, repo, "watched");
    let before = reads(repo);
    Watched {
        repo: repo.to_owned(),
        planted,
        before,
    }
}

impl Watched {
    /// The reads are as they were, and both planted objects are still there.
    fn assert_unchanged(&self) {
        assert_unchanged(&self.repo, &self.before, &self.planted);
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

/// The steps a loop of calls has ended, counted by every thread that takes part in it.
#[derive(Clone, Default)]
struct Progress(Arc<AtomicUsize>);

impl Progress {
    /// Notes that one step ended.
    fn step(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }

    fn steps(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

/// Runs `work` on a thread of its own and waits for its end as long as a step of it ends
/// within [`STEP_DEADLINE`]: a deadlock is a failure that says so, not a test that never
/// ends, and a slow machine is not a failure.
fn ends_in_time(what: &str, work: impl FnOnce(&Progress) + Send + 'static) {
    let (done, ended) = channel();
    let progress = Progress::default();
    let counted = progress.clone();
    std::thread::spawn(move || {
        work(&counted);
        let _ = done.send(());
    });
    loop {
        let seen = progress.steps();
        match ended.recv_timeout(STEP_DEADLINE) {
            Ok(()) => return,
            Err(RecvTimeoutError::Timeout) if progress.steps() != seen => {}
            Err(RecvTimeoutError::Timeout) => panic!("{what} did not end: a deadlock"),
            Err(RecvTimeoutError::Disconnected) => panic!("{what} failed in a thread"),
        }
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
    let watched = watch(&scratch, &repo);
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
    watched.assert_unchanged();
}

#[test]
fn a_state_recorded_after_the_caller_listed_the_states_is_not_deleted() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, "repo");
    let watched = watch(&scratch, &repo);
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
    watched.assert_unchanged();
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
    let watched = watch(&scratch, &repo);
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
    watched.assert_unchanged();
}

#[test]
fn a_delete_takes_away_only_refs_of_states_and_kept_commits() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, "repo");
    let watched = watch(&scratch, &repo);
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
    watched.assert_unchanged();
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
    let watched = watch(&scratch, &repo);
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
    watched.assert_unchanged();
}

#[test]
fn packing_a_store_with_nothing_in_it_or_nothing_deleted_changes_no_state() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, "repo");
    let watched = watch(&scratch, &repo);
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
    watched.assert_unchanged();
}

#[test]
fn the_pack_removes_the_complete_head_when_its_commit_went_and_the_next_recording_copies_it_whole()
{
    let scratch = Scratch::new();
    let repo = repository(&scratch, "repo");
    let mut noise = Noise(17);
    std::fs::write(repo.join("committed.bin"), noise.kib(16)).expect("write");
    // Only that file: `turn.bin` stays untracked, so that the test's own rewriting of it
    // leaves the repository's reads as they were.
    git(&repo, &["add", "committed.bin"]);
    git(&repo, &["commit", "--quiet", "-m", "two"]);
    let watched = watch(&scratch, &repo);
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
    watched.assert_unchanged();
}

#[test]
fn the_pack_keeps_the_complete_head_while_a_state_still_reaches_its_commit() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, "repo");
    let watched = watch(&scratch, &repo);
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
    watched.assert_unchanged();
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
    watched: Watched,
}

fn stressed(scratch: &Scratch) -> Stressed {
    let repo = repository(scratch, "repo");
    let mut noise = Noise(29);
    std::fs::write(repo.join("old.bin"), noise.kib(8)).expect("write");
    // `turn.bin` stays untracked, so that the tests' own rewriting of it leaves the
    // repository's reads as they were.
    git(&repo, &["add", "old.bin"]);
    git(&repo, &["commit", "--quiet", "-m", "two"]);
    let old_commit = git(&repo, &["rev-parse", "HEAD"]);
    write(&repo, "notes.txt", "one\nthree\n");
    std::fs::remove_file(repo.join("old.bin")).expect("remove");
    git(&repo, &["commit", "--quiet", "-a", "-m", "three"]);
    let watched = watch(scratch, &repo);
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
        watched,
    }
}

/// Deletes the kept commit and packs, `rounds` times, as the clean-up at once does.
fn clean_up_repeatedly(store: &StateStore, commit: &str, rounds: usize, progress: &Progress) {
    let ids: BTreeSet<String> = [commit.to_owned()].into();
    for _ in 0..rounds {
        store.delete(&ids).expect("delete");
        progress.step();
        store.pack().expect("pack");
        progress.step();
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
        watched,
    } = stressed(&scratch);
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

    ends_in_time("the comparisons and the clean-ups", move |progress| {
        let cleaner = {
            let (store, commit, progress) = (store.clone(), old_commit.clone(), progress.clone());
            std::thread::spawn(move || clean_up_repeatedly(&store, &commit, 50, &progress))
        };
        // Two threads compare while it runs: the first against a commit of the repository
        // that the store copies and the clean-up lets go, the second against the files on
        // disk.
        let with_commit = {
            let (store, worktree) = (store.clone(), worktree.clone());
            let (commit, state) = (old_commit.clone(), state.id.to_string());
            let progress = progress.clone();
            std::thread::spawn(move || {
                for compared in 0..200 {
                    let answer = store
                        .compare(Some(&worktree), &commit, Some(&state))
                        .unwrap_or_else(|e| panic!("comparison {compared} failed: {e}"));
                    assert_eq!(answer, against_commit, "comparison {compared}");
                    progress.step();
                }
            })
        };
        for compared in 0..200 {
            let answer = store
                .compare(Some(&worktree), state.id.as_str(), None)
                .unwrap_or_else(|e| panic!("comparison {compared} with the disk failed: {e}"));
            assert_eq!(answer, against_disk, "comparison {compared} with the disk");
            progress.step();
        }
        with_commit.join().expect("the comparisons");
        cleaner.join().expect("the clean-ups");
    });
    watched.assert_unchanged();
}

#[test]
fn every_call_that_holds_a_version_works_beside_the_clean_up_and_none_waits_forever() {
    let scratch = Scratch::new();
    let Stressed {
        worktree,
        store,
        state,
        old_commit,
        watched,
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

    ends_in_time("the calls and the clean-ups", move |progress| {
        let cleaner = {
            let (store, commit, progress) = (store.clone(), old_commit.clone(), progress.clone());
            std::thread::spawn(move || clean_up_repeatedly(&store, &commit, 25, &progress))
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
            progress.step();
        }
        cleaner.join().expect("the clean-ups");
    });
    watched.assert_unchanged();
}

#[test]
fn a_ref_moved_after_the_delete_read_the_store_is_not_deleted_and_no_other_ref_is() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, "repo");
    let watched = watch(&scratch, &repo);
    let worktree = open(&repo);
    let clock = TestClock::new();
    let folder = scratch.join("states/one.git");
    // A git that, once armed, moves one ref of the store just before the store's own
    // `update-ref --stdin` runs: another handle recording meanwhile.
    let armed = scratch.join("move-a-ref");
    let moving = scratch.join("bin/git-moving");
    program(
        &moving,
        &format!(
            "case \"$*\" in\n*\"update-ref --stdin\"*)\n  if [ -e '{armed}' ]; then\n    \
             read name object < '{armed}'\n    rm '{armed}'\n    \
             git --git-dir='{folder}' update-ref \"$name\" \"$object\"\n  fi;;\nesac\n\
             exec git \"$@\"",
            armed = armed.display(),
            folder = folder.display(),
        ),
    );
    let store = open_store_through(&moving, &folder, &clock);
    let mut noise = Noise(41);
    let first = record_with_noise(&store, &repo, &worktree, &mut noise);
    let second = record_with_noise(&store, &repo, &worktree, &mut noise);
    let name_of = |state: &RecordedState| {
        git(
            store.folder(),
            &[
                "for-each-ref",
                "--points-at",
                state.id.as_str(),
                "--format=%(refname)",
                "refs/yaar/states/",
            ],
        )
    };
    let (ref_1, ref_2) = (name_of(&first), name_of(&second));
    // The second state's ref now names the first state's commit, as if it had been deleted
    // and the number taken again by a state of another handle.
    std::fs::write(&armed, format!("{ref_2} {}\n", first.id)).expect("arm the git");

    let both: BTreeSet<String> = [&first, &second]
        .map(|state| state.id.as_str().to_owned())
        .into();
    let refused = store.delete(&both);

    assert!(
        refused.is_err(),
        "a ref that moved is a delete that fails: {refused:?}"
    );
    assert!(!armed.exists(), "the ref was moved before the delete ran");
    assert_eq!(
        refs_of(store.folder()),
        format!("{id} commit\t{ref_1}\n{id} commit\t{ref_2}", id = first.id),
        "the moved ref still names what it was moved to, and the other ref is not deleted either"
    );
    watched.assert_unchanged();
}

/// Makes a named pipe and opens it for reading and writing, so that neither end waits for
/// the other to open: a read waits for a byte, a write leaves one.
fn pipe(path: &Path) -> File {
    let made = Command::new("mkfifo").arg(path).status().expect("mkfifo");
    assert!(made.success(), "mkfifo {path:?}");
    File::options()
        .read(true)
        .write(true)
        .open(path)
        .expect("open the pipe")
}

/// What [`clean_up_beside_a_call`] leaves for the test to read afterwards.
struct Beside<T> {
    /// What the call answered once it was let go.
    answer: T,
    repo: PathBuf,
    store: Arc<StateStore>,
    /// The repository's three commits, the oldest first.
    commits: [String; 3],
    watched: Watched,
}

/// A repository of three commits and a store whose git stops once, at the command whose name
/// is `stop_at`: `call` is made on a thread of its own, holds a commit of the store and is
/// stopped at that command, before it has read; the commits `released` are then deleted and
/// packed beside it, and must not be gone while it waits. Once it is let go it reads what it
/// held, and the clean-up ends after it. The commits `kept` are kept in the store first.
///
/// What the test waits on is the git's own word that the call has arrived and the call's
/// answer. The one wait that is not on a fact is [`SETTLE`], which shows a negative: a
/// clean-up that is blocked leaves nothing to look at.
fn clean_up_beside_a_call<T: Send + 'static>(
    scratch: &Scratch,
    stop_at: &str,
    kept: &[usize],
    released: &[usize],
    call: impl FnOnce(&StateStore, &Worktree, &[String; 3]) -> T + Send + 'static,
) -> Beside<T> {
    let repo = repository(scratch, "repo");
    let first = git(&repo, &["rev-parse", "HEAD"]);
    let mut noise = Noise(37);
    std::fs::write(repo.join("unique.bin"), noise.kib(8)).expect("write");
    let second = commit_all(&repo, "two");
    write(&repo, "notes.txt", "changed\n");
    let third = commit_all(&repo, "three");
    let commits = [first, second, third];
    let watched = watch(scratch, &repo);
    let worktree = Arc::new(open(&repo));
    // This git stops, once armed, at the command, tells the test and waits for it to say go.
    let armed = scratch.join("stop-the-call");
    let (reached_path, go_path) = (scratch.join("reached"), scratch.join("go"));
    let stopping = scratch.join("bin/git-stopping");
    program(
        &stopping,
        &format!(
            "case \" $* \" in\n*\" {stop_at} \"*)\n  if [ -e '{armed}' ]; then\n    \
             rm '{armed}'\n    echo reached > '{reached}'\n    read go < '{go}'\n  fi;;\nesac\n\
             exec git \"$@\"",
            armed = armed.display(),
            reached = reached_path.display(),
            go = go_path.display(),
        ),
    );
    let (mut reached, mut go) = (pipe(&reached_path), pipe(&go_path));
    let clock = TestClock::new();
    let store = Arc::new(open_store_through(
        &stopping,
        &scratch.join("states/one.git"),
        &clock,
    ));
    for index in kept {
        store
            .keep_commit(&worktree, &CommitId::new(commits[*index].clone()))
            .expect("keep the commit");
    }
    std::fs::write(&armed, "").expect("arm the git");

    let calling = {
        let (store, worktree, commits) = (store.clone(), worktree.clone(), commits.clone());
        std::thread::spawn(move || call(&store, &worktree, &commits))
    };
    // The call holds what it reads, in the store and named by a ref, and has not read.
    let (told, arrived) = channel();
    std::thread::spawn(move || {
        let mut byte = [0u8; 1];
        let _ = told.send(reached.read_exact(&mut byte));
    });
    arrived
        .recv_timeout(DEADLINE)
        .expect("the call reached the command")
        .expect("read the pipe");

    // The clean-up starts now. It must not end while the call is between holding the commits
    // and reading them.
    let (cleaned, ended) = channel();
    let cleaner = {
        let store = store.clone();
        let ids: BTreeSet<String> = released
            .iter()
            .map(|index| commits[*index].clone())
            .collect();
        std::thread::spawn(move || {
            store.delete(&ids).expect("delete");
            store.pack().expect("pack");
            let _ = cleaned.send(());
        })
    };
    let ran_on = ended.recv_timeout(SETTLE);
    go.write_all(b"go\n").expect("say go");
    assert_eq!(
        ran_on,
        Err(RecvTimeoutError::Timeout),
        "the clean-up ended while a call held the commits"
    );

    let answer = calling.join().expect("the call");
    ended
        .recv_timeout(DEADLINE)
        .expect("the clean-up ended once the call had");
    cleaner.join().expect("the clean-up");
    Beside {
        answer,
        repo,
        store,
        commits,
        watched,
    }
}

#[test]
fn a_clean_up_waits_for_a_read_that_holds_a_version_until_it_has_read() {
    let scratch = Scratch::new();
    let held = clean_up_beside_a_call(
        &scratch,
        "cat-file --batch",
        &[],
        &[1],
        |store, worktree, commits| {
            // The first read of a file starts the process that hands out contents, after the
            // commit was copied into the store and before anything is read.
            store.content_in(Some(worktree), &commits[1], Path::new("unique.bin"))
        },
    );
    let read = held.answer.expect("content_in");
    assert_eq!(
        read,
        Some(std::fs::read(held.repo.join("unique.bin")).expect("the file")),
        "the read found what it held"
    );
    assert!(
        held.store.paths(&held.commits[1]).is_err(),
        "and then the clean-up took the commit away"
    );
    held.watched.assert_unchanged();
}

#[test]
fn a_clean_up_waits_for_a_call_that_lists_the_paths_of_a_kept_commit_until_it_has_listed_them() {
    let scratch = Scratch::new();
    let held = clean_up_beside_a_call(&scratch, "ls-tree", &[1], &[1], |store, _, commits| {
        store.paths(&commits[1])
    });
    let expected: Vec<PathBuf> = git(
        &held.repo,
        &["ls-tree", "-r", "--name-only", &held.commits[1]],
    )
    .lines()
    .map(PathBuf::from)
    .collect();
    assert!(expected.len() > 1, "the commit holds files");
    assert_eq!(held.answer.expect("paths"), expected);
    assert!(
        held.store.paths(&held.commits[1]).is_err(),
        "and then the clean-up took the commit away"
    );
    held.watched.assert_unchanged();
}

#[test]
fn a_clean_up_waits_for_a_call_that_compares_two_kept_commits_until_it_has_compared_them() {
    let scratch = Scratch::new();
    let held = clean_up_beside_a_call(
        &scratch,
        "diff-tree",
        &[1, 2],
        &[1, 2],
        |store, _, commits| store.changed_paths(&commits[1], &commits[2]),
    );
    assert_eq!(
        held.answer.expect("changed_paths"),
        vec![ChangedPath {
            path: PathBuf::from("notes.txt"),
            change: Change::Modified,
        }]
    );
    assert!(
        held.store
            .changed_paths(&held.commits[1], &held.commits[2])
            .is_err(),
        "and then the clean-up took the commits away"
    );
    held.watched.assert_unchanged();
}

/// The process ids of the starts of the store's reader of contents, from the list of a git
/// that writes every start of itself down, oldest first.
fn reader_starts(list: &Path) -> Vec<String> {
    std::fs::read_to_string(list)
        .unwrap_or_default()
        .lines()
        .filter(|line| line.contains("cat-file --batch -Z"))
        .map(|line| line.split(' ').next().expect("a process id").to_owned())
        .collect()
}

/// The packs the store holds, by name.
fn packs_of(store: &Path) -> BTreeSet<OsString> {
    std::fs::read_dir(store.join("objects/pack"))
        .expect("read the folder of packs")
        .map(|entry| entry.expect("an entry").file_name())
        .filter(|name| name.to_string_lossy().ends_with(".pack"))
        .collect()
}

/// The processes of the store's reader of contents that this test process started and that
/// still run: its children whose command line is `git cat-file --batch`. A child exists
/// before the call that started it returns, so this holds a fact of the moment it is asked
/// in, which a note the child writes itself does not (it writes after it was started).
fn readers_running() -> Vec<u32> {
    let me = std::process::id();
    let is_reader = |command: &str| command.contains("cat-file --batch");
    if cfg!(target_os = "linux") {
        std::fs::read_dir("/proc")
            .expect("read /proc")
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<u32>().ok())
            .filter(|pid| {
                let parent = std::fs::read_to_string(format!("/proc/{pid}/stat"))
                    .ok()
                    .and_then(|stat| {
                        // The parent is the second field after the last `)`.
                        let (_, rest) = stat.rsplit_once(')')?;
                        rest.split_whitespace().nth(1)?.parse::<u32>().ok()
                    });
                // The arguments are separated by NUL bytes.
                parent == Some(me)
                    && std::fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|bytes| {
                        is_reader(&String::from_utf8_lossy(&bytes).replace('\0', " "))
                    })
            })
            .collect()
    } else {
        let listing = Command::new("ps")
            .args(["-axo", "pid=,ppid=,command="])
            .output()
            .expect("start ps");
        assert!(listing.status.success(), "ps failed");
        String::from_utf8_lossy(&listing.stdout)
            .lines()
            .filter_map(|line| {
                let (pid, rest) = line.trim_start().split_once(' ')?;
                let (parent, command) = rest.trim_start().split_once(' ')?;
                if parent.parse::<u32>().ok()? == me && is_reader(command) {
                    pid.parse::<u32>().ok()
                } else {
                    None
                }
            })
            .collect()
    }
}

/// Waits on a thread of its own for one byte of the pipe and tells the channel it came.
fn byte_of(mut pipe: File) -> std::sync::mpsc::Receiver<std::io::Result<()>> {
    let (told, came) = channel();
    std::thread::spawn(move || {
        let mut byte = [0u8; 1];
        let _ = told.send(pipe.read_exact(&mut byte));
    });
    came
}

#[test]
fn a_pack_that_removes_a_pack_waits_for_a_read_in_flight_before_it_ends_the_reader() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, "repo");
    let watched = watch(&scratch, &repo);
    let worktree = open(&repo);
    let clock = TestClock::new();
    // This git writes down every start of itself. Once armed, it tells the test at the
    // start of `prune`, and stops at the start of the reader of contents, which the first
    // read has by then asked for under the lock a read takes, until the test says go.
    let list = scratch.join("starts-of-git");
    let (pack_armed, read_armed) = (scratch.join("arm-the-prune"), scratch.join("arm-the-read"));
    let (pruning_path, reached_path, go_path) = (
        scratch.join("pruning"),
        scratch.join("reached"),
        scratch.join("go"),
    );
    let stopping = scratch.join("bin/git-listed-and-stopping");
    program(
        &stopping,
        &format!(
            "printf '%s\\n' \"$$ $*\" >> '{list}'\n\
             case \" $* \" in\n\
             *\" prune \"*)\n  if [ -e '{pack_armed}' ]; then\n    rm '{pack_armed}'\n    \
             echo pruning > '{pruning}'\n  fi;;\n\
             *\" cat-file --batch \"*)\n  if [ -e '{read_armed}' ]; then\n    rm '{read_armed}'\n    \
             echo reached > '{reached}'\n    read go < '{go}'\n  fi;;\nesac\n\
             exec git \"$@\"",
            list = list.display(),
            pack_armed = pack_armed.display(),
            read_armed = read_armed.display(),
            pruning = pruning_path.display(),
            reached = reached_path.display(),
            go = go_path.display(),
        ),
    );
    let (pruning, reached, mut go) = (pipe(&pruning_path), pipe(&reached_path), pipe(&go_path));
    let store = Arc::new(open_store_through(
        &stopping,
        &scratch.join("states/one.git"),
        &clock,
    ));
    let mut noise = Noise(47);
    let first = record_with_noise(&store, &repo, &worktree, &mut noise);
    let second = record_with_noise(&store, &repo, &worktree, &mut noise);
    let kept_bytes = std::fs::read(repo.join("turn.bin")).expect("what the second state holds");
    store.pack().expect("pack what the recordings wrote loose");
    record_with_noise(&store, &repo, &worktree, &mut noise);
    let packs = packs_of(store.folder());
    assert!(reader_starts(&list).is_empty(), "no read has asked yet");

    // The first read of a state asks for the reader and stops at its start, holding the lock
    // of the reader.
    std::fs::write(&read_armed, "").expect("arm the read");
    let reading = {
        let (store, id) = (store.clone(), second.id.to_string());
        std::thread::spawn(move || store.content(&id, Path::new("turn.bin")))
    };
    byte_of(reached)
        .recv_timeout(DEADLINE)
        .expect("the read reached the start of the reader")
        .expect("read the pipe");
    let started = reader_starts(&list);
    assert_eq!(
        started.len(),
        1,
        "the read asked for one reader: {started:?}"
    );

    // A pack that removes the old pack runs beside it, up to the start of its prune.
    std::fs::write(&pack_armed, "").expect("arm the prune");
    let (cleaned, ended) = channel();
    let cleaner = {
        let (store, ids) = (store.clone(), BTreeSet::from([first.id.to_string()]));
        std::thread::spawn(move || {
            store.delete(&ids).expect("delete");
            store.pack().expect("pack");
            let _ = cleaned.send(());
        })
    };
    byte_of(pruning)
        .recv_timeout(DEADLINE)
        .expect("the pack reached its prune")
        .expect("read the pipe");
    // The one timed negative: a pack that does not wait for the read leaves nothing to look
    // at, as `clean_up_beside_a_call` says. A passing run cannot fail on it: the read is let
    // go only after the window.
    let ran_on = ended.recv_timeout(SETTLE);
    go.write_all(b"go\n").expect("say go");
    assert_eq!(
        ran_on,
        Err(RecvTimeoutError::Timeout),
        "the pack ended while a read of the reader was in flight"
    );

    let read = reading.join().expect("the read").expect("read");
    assert_eq!(
        read,
        Some(kept_bytes.clone()),
        "the read answered its bytes"
    );
    ended
        .recv_timeout(DEADLINE)
        .expect("the pack ended once the read had");
    cleaner.join().expect("the clean-up");
    assert!(
        !packs_of(store.folder()).is_superset(&packs),
        "the pack removed the old pack"
    );
    assert!(
        !is_alive(&started[0]),
        "the reader the read started ended with the pack, and was waited for"
    );
    assert_eq!(
        readers_running(),
        Vec::<u32>::new(),
        "none starts before a read asks"
    );

    // The next read starts another reader, and the bytes are the same.
    assert_eq!(
        store
            .content(second.id.as_str(), Path::new("turn.bin"))
            .expect("read"),
        Some(kept_bytes),
        "the same bytes"
    );
    let starts = reader_starts(&list);
    assert_eq!(starts.len(), 2, "one more start: {starts:?}");
    assert_ne!(starts[1], started[0]);
    assert!(
        is_alive(&starts[1]),
        "the new reader waits for the next read"
    );
    watched.assert_unchanged();
}

#[test]
fn the_reader_of_contents_ends_at_the_end_of_a_pack_that_removed_a_pack_and_the_next_read_starts_one_anew()
 {
    let scratch = Scratch::new();
    let repo = repository(&scratch, "repo");
    let watched = watch(&scratch, &repo);
    let worktree = open(&repo);
    let clock = TestClock::new();
    let (noting, list) = git_that_keeps_a_list(&scratch);
    let store = open_store_through(&noting, &scratch.join("states/one.git"), &clock);
    let mut noise = Noise(41);
    let first = record_with_noise(&store, &repo, &worktree, &mut noise);
    let second = record_with_noise(&store, &repo, &worktree, &mut noise);
    store.pack().expect("pack what the recordings wrote loose");
    let path = Path::new("turn.bin");
    let before = store
        .content(second.id.as_str(), path)
        .expect("read")
        .expect("a file");
    let started = reader_starts(&list);
    assert_eq!(
        started.len(),
        1,
        "one reader started by the first read: {started:?}"
    );
    let old_reader = started[0].clone();
    assert!(is_alive(&old_reader), "the reader waits for the next read");

    // A pack that removes nothing ends nothing.
    let packs = packs_of(store.folder());
    store.pack().expect("pack again");
    assert_eq!(packs_of(store.folder()), packs, "this pack removed no pack");
    assert!(
        is_alive(&old_reader),
        "a pack that removed nothing ended the reader"
    );
    assert_eq!(reader_starts(&list), std::slice::from_ref(&old_reader));

    // One that removes the old pack ends it, and waits for it: the room is given back.
    record_with_noise(&store, &repo, &worktree, &mut noise);
    store
        .delete(&[first.id.to_string()].into())
        .expect("delete");
    store.pack().expect("pack");
    assert!(
        !packs_of(store.folder()).is_superset(&packs),
        "the pack removed the old pack"
    );
    assert!(
        !is_alive(&old_reader),
        "the reader that had the old pack mapped ended with the pack, and was waited for"
    );
    assert_eq!(
        readers_running(),
        Vec::<u32>::new(),
        "none starts before a read asks"
    );

    // The next read starts a new one and reads what the old one did.
    let after = store
        .content(second.id.as_str(), path)
        .expect("read")
        .expect("a file");
    assert_eq!(after, before, "the same bytes");
    let starts = reader_starts(&list);
    assert_eq!(starts.len(), 2, "one more start: {starts:?}");
    assert_ne!(starts[1], old_reader);
    assert!(
        is_alive(&starts[1]),
        "the new reader waits for the next read"
    );
    watched.assert_unchanged();

    drop(store);
    assert!(!is_alive(&starts[1]), "it ends with the store");
}

#[test]
fn the_reader_of_contents_ends_at_the_end_of_a_pack_that_only_pruned_loose_objects() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, "repo");
    let watched = watch(&scratch, &repo);
    let worktree = open(&repo);
    let clock = TestClock::new();
    let (noting, list) = git_that_keeps_a_list(&scratch);
    let store = open_store_through(&noting, &scratch.join("states/one.git"), &clock);
    let mut noise = Noise(43);
    let state = record_with_noise(&store, &repo, &worktree, &mut noise);
    store.pack().expect("pack what the recording wrote loose");
    let path = Path::new("turn.bin");
    let before = store
        .content(state.id.as_str(), path)
        .expect("read")
        .expect("a file");
    let started = reader_starts(&list);
    assert_eq!(started.len(), 1, "one reader: {started:?}");

    // A comparison with the files on disk writes a tree and a blob no ref names.
    write(&repo, "notes.txt", "changed since the state\n");
    store
        .compare(Some(&worktree), state.id.as_str(), None)
        .expect("compare");
    write(&repo, "notes.txt", "one\ntwo\nthree\n");
    let packs = packs_of(store.folder());
    store.pack().expect("pack");

    assert_eq!(packs_of(store.folder()), packs, "this pack removed no pack");
    assert!(
        !is_alive(&started[0]),
        "the pack that pruned objects ended the reader, and waited for it"
    );
    assert_eq!(
        store.content(state.id.as_str(), path).expect("read"),
        Some(before),
        "the same bytes from the next reader"
    );
    assert_eq!(reader_starts(&list).len(), 2, "the next read started one");
    watched.assert_unchanged();
}

#[test]
fn the_reader_of_contents_ends_with_a_pack_that_removed_a_pack_though_its_prune_then_failed() {
    let scratch = Scratch::new();
    let repo = repository(&scratch, "repo");
    let watched = watch(&scratch, &repo);
    let worktree = open(&repo);
    let clock = TestClock::new();
    // This git writes down every start of itself and, once armed, fails the next `prune`
    // after `repack` ran: the stand-in for a disk that fails then.
    let list = scratch.join("starts-of-git");
    let armed = scratch.join("fail-the-prune");
    let failing = scratch.join("bin/git-listed-and-failing");
    program(
        &failing,
        &format!(
            "printf '%s\\n' \"$$ $*\" >> '{list}'\n\
             case \" $* \" in\n\
             *\" prune \"*)\n  if [ -e '{armed}' ]; then\n    rm '{armed}'\n    \
             echo 'the prune failed' >&2\n    exit 1\n  fi;;\nesac\n\
             exec git \"$@\"",
            list = list.display(),
            armed = armed.display(),
        ),
    );
    let store = open_store_through(&failing, &scratch.join("states/one.git"), &clock);
    let mut noise = Noise(47);
    let first = record_with_noise(&store, &repo, &worktree, &mut noise);
    let second = record_with_noise(&store, &repo, &worktree, &mut noise);
    store.pack().expect("pack what the recordings wrote loose");
    let path = Path::new("turn.bin");
    let before = store
        .content(second.id.as_str(), path)
        .expect("read")
        .expect("a file");
    let started = reader_starts(&list);
    assert_eq!(started.len(), 1, "one reader: {started:?}");
    let packs = packs_of(store.folder());

    // A pack that removes the old pack, and whose prune then fails.
    record_with_noise(&store, &repo, &worktree, &mut noise);
    store
        .delete(&[first.id.to_string()].into())
        .expect("delete");
    std::fs::write(&armed, "").expect("arm the failing prune");
    let failed = store.pack();
    assert!(failed.is_err(), "the prune's failure is given: {failed:?}");
    assert!(
        !packs_of(store.folder()).is_superset(&packs),
        "the pack removed the old pack before the prune failed"
    );
    assert!(
        !is_alive(&started[0]),
        "the reader that had the removed pack mapped ended, and was waited for"
    );
    assert_eq!(
        readers_running(),
        Vec::<u32>::new(),
        "none starts before a read asks"
    );

    // The next read starts a new one and gives the bytes the old one gave.
    assert_eq!(
        store.content(second.id.as_str(), path).expect("read"),
        Some(before),
        "the same bytes from the next reader"
    );
    assert_eq!(reader_starts(&list).len(), 2, "the next read started one");
    watched.assert_unchanged();
}

//! Component tests: the contents of files in recorded states, read through one process a
//! state store keeps running, in real repositories the tests make.

mod support;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use support::{
    Scratch, an_older_git, commit_all, git_that_keeps_a_list, init, is_alive, program, write,
};
use yaar_recorder::{CommitId, Git, RecordedState, StateStore, Worktree};

/// How long a test waits for a fact that is not in the test's hands.
const DEADLINE: Duration = Duration::from_secs(10);

/// The lines of the list that are starts of the process that hands out contents, as
/// process ids. Other runs of `cat-file` (a check that an object is there) are not it.
fn content_processes(list: &Path) -> Vec<String> {
    std::fs::read_to_string(list)
        .unwrap_or_default()
        .lines()
        // Of a git that has `-Z` and of one that has not; `--batch-check` is another process.
        .filter(|line| line.ends_with("cat-file --batch -Z") || line.ends_with("cat-file --batch"))
        .map(|line| line.split(' ').next().expect("a process id").to_owned())
        .collect()
}

fn content_of(n: usize, version: &str) -> Vec<u8> {
    format!("content {n}, {version}\n").into_bytes()
}

/// A repository of 20 files: `first` commit holds them as "first", the `second` commit
/// changes file 0.
struct Repository {
    worktree: Worktree,
    first: String,
    second: String,
}

fn repository(scratch: &Scratch) -> Repository {
    let repo = scratch.join("repo");
    init(&repo);
    for n in 0..20 {
        write(
            &repo,
            &format!("file-{n}.txt"),
            &String::from_utf8(content_of(n, "first")).unwrap(),
        );
    }
    let first = commit_all(&repo, "start");
    write(
        &repo,
        "file-0.txt",
        &String::from_utf8(content_of(0, "second")).unwrap(),
    );
    let second = commit_all(&repo, "a change");
    Repository {
        worktree: Worktree::open(&Git::find().expect("git"), &repo).expect("open"),
        first,
        second,
    }
}

/// A store in the scratch folder whose every start of git is written to the returned list.
fn listed_store(scratch: &Scratch) -> (StateStore, PathBuf) {
    let (program, list) = git_that_keeps_a_list(scratch);
    let store = StateStore::open(&Git::at(program), &scratch.join("app/states/one.git"))
        .expect("open the store");
    (store, list)
}

fn is_dead(pid: &str) -> bool {
    !is_alive(pid)
}

#[test]
fn forty_reads_of_two_states_and_a_kept_commit_start_one_git_process_which_ends_with_the_store() {
    forty_reads_of_two_states_and_a_kept_commit_start_one_git_process_which_ends_with_the_store_with(&Scratch::new());
}

#[test]
fn forty_reads_of_two_states_and_a_kept_commit_start_one_git_process_which_ends_with_the_store_on_an_older_git()
 {
    let scratch = Scratch::new();
    an_older_git(&scratch);
    forty_reads_of_two_states_and_a_kept_commit_start_one_git_process_which_ends_with_the_store_with(&scratch);
}

fn forty_reads_of_two_states_and_a_kept_commit_start_one_git_process_which_ends_with_the_store_with(
    scratch: &Scratch,
) {
    let Repository {
        worktree,
        first,
        second,
    } = repository(scratch);
    let (store, list) = listed_store(scratch);
    let state_one = store.record(&worktree).expect("record");
    write(
        worktree.root(),
        "file-1.txt",
        &String::from_utf8(content_of(1, "third")).unwrap(),
    );
    let state_two = store.record(&worktree).expect("record again");
    store
        .keep_commit(&worktree, &CommitId::new(&first))
        .expect("keep");
    assert_ne!(state_one.id, state_two.id);
    assert_eq!(
        content_processes(&list),
        Vec::<String>::new(),
        "none before a read"
    );

    // 20 files in each of three commits, and one that is in none of them: 63 reads.
    for (commit, file_1, file_0) in [
        (first.as_str(), "first", "first"),
        (second.as_str(), "first", "second"),
        (state_one.id.as_str(), "first", "second"),
        (state_two.id.as_str(), "third", "second"),
    ] {
        for n in 0..20 {
            let expected = match n {
                0 => content_of(0, file_0),
                1 => content_of(1, file_1),
                n => content_of(n, "first"),
            };
            let path = format!("file-{n}.txt");
            // The commit the state was taken on and the kept commit are held by the store.
            assert_eq!(
                store.content(commit, Path::new(&path)).expect("content"),
                Some(expected),
                "{path} in {commit}"
            );
        }
        assert_eq!(
            store
                .content(commit, Path::new("none.txt"))
                .expect("content"),
            None
        );
    }
    // The way a version is named when it may need copying in reads through the same process.
    assert_eq!(
        store
            .content_in(None, state_two.id.as_str(), Path::new("file-1.txt"))
            .expect("content_in"),
        Some(content_of(1, "third"))
    );

    let started = content_processes(&list);
    assert_eq!(started.len(), 1, "one process for 85 reads: {started:?}");
    assert!(is_alive(&started[0]), "it stays for the next read");

    drop(store);
    assert!(is_dead(&started[0]), "it ended, and was waited for");
}

#[test]
fn a_process_for_contents_that_died_is_started_again() {
    a_process_for_contents_that_died_is_started_again_with(&Scratch::new());
}

#[test]
fn a_process_for_contents_that_died_is_started_again_on_an_older_git() {
    let scratch = Scratch::new();
    an_older_git(&scratch);
    a_process_for_contents_that_died_is_started_again_with(&scratch);
}

fn a_process_for_contents_that_died_is_started_again_with(scratch: &Scratch) {
    let repository = repository(scratch);
    let (store, list) = listed_store(scratch);
    let state = store.record(&repository.worktree).expect("record");
    let read = || {
        store
            .content(state.id.as_str(), Path::new("file-3.txt"))
            .expect("content")
    };

    assert_eq!(read(), Some(content_of(3, "first")));
    let first = content_processes(&list);
    let killed = std::process::Command::new("sh")
        .args(["-c", &format!("kill -9 {}", first[0])])
        .status()
        .expect("start the shell");
    assert!(killed.success());

    assert_eq!(read(), Some(content_of(3, "first")));
    assert_eq!(read(), Some(content_of(3, "first")));
    let started = content_processes(&list);
    assert_eq!(started.len(), 2, "one more process, for every later read");
    assert_ne!(started[0], started[1]);
    assert!(is_dead(&started[0]), "the dead one was waited for");
    assert!(is_alive(&started[1]));

    drop(store);
    assert!(is_dead(&started[1]));
}

#[test]
fn several_threads_read_through_one_store() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    for n in 0..8 {
        write(
            &repo,
            &format!("file-{n}.txt"),
            &format!("content {n}\n").repeat(500),
        );
    }
    commit_all(&repo, "start");
    let worktree = Worktree::open(&Git::find().expect("git"), &repo).expect("open");
    let (store, list) = listed_store(&scratch);
    let state = store.record(&worktree).expect("record");

    std::thread::scope(|threads| {
        for n in 0..8 {
            let (store, state) = (&store, &state);
            threads.spawn(move || {
                for _ in 0..25 {
                    let path = format!("file-{n}.txt");
                    assert_eq!(
                        store
                            .content(state.id.as_str(), Path::new(&path))
                            .expect("content"),
                        Some(format!("content {n}\n").repeat(500).into_bytes())
                    );
                }
            });
        }
    });

    assert_eq!(content_processes(&list).len(), 1, "one for every thread");
}

#[test]
fn what_the_store_comes_to_hold_after_the_process_started_is_read_by_that_same_process() {
    what_the_store_comes_to_hold_after_the_process_started_is_read_by_that_same_process_with(
        &Scratch::new(),
    );
}

#[test]
fn what_the_store_comes_to_hold_after_the_process_started_is_read_by_that_same_process_on_an_older_git()
 {
    let scratch = Scratch::new();
    an_older_git(&scratch);
    what_the_store_comes_to_hold_after_the_process_started_is_read_by_that_same_process_with(
        &scratch,
    );
}

fn what_the_store_comes_to_hold_after_the_process_started_is_read_by_that_same_process_with(
    scratch: &Scratch,
) {
    let Repository {
        worktree,
        first,
        second: _,
    } = repository(scratch);
    let (store, list) = listed_store(scratch);
    let state_one = store.record(&worktree).expect("record");
    let file_0 = Path::new("file-0.txt");

    // The store does not hold the first commit: the process is started and finds nothing.
    assert_eq!(store.content(&first, file_0).expect("content"), None);
    // A state taken after the process started is read by it.
    write(worktree.root(), "file-0.txt", "changed once more\n");
    let state_two = store.record(&worktree).expect("record again");
    assert_eq!(
        store.content(state_two.id.as_str(), file_0).expect("state"),
        Some(b"changed once more\n".to_vec())
    );
    // So is a commit copied in after it was asked for and not found.
    assert_eq!(
        store
            .content_in(Some(&worktree), &first, file_0)
            .expect("content_in"),
        Some(content_of(0, "first"))
    );
    assert_eq!(
        store.content(&first, file_0).expect("kept"),
        Some(content_of(0, "first"))
    );
    assert_eq!(
        store.content(state_one.id.as_str(), file_0).expect("old"),
        Some(content_of(0, "second"))
    );
    assert_eq!(content_processes(&list).len(), 1);
}

/// A FIFO of the scratch folder that the test itself holds open for reading and writing,
/// so that a program opening either end never waits for the test, and the test never
/// waits for the program to open.
struct Pipe {
    file: std::fs::File,
    path: PathBuf,
}

impl Pipe {
    fn new(path: PathBuf) -> Pipe {
        let made = std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .expect("start mkfifo");
        assert!(made.success(), "mkfifo");
        let file = std::fs::File::options()
            .read(true)
            .write(true)
            .open(&path)
            .expect("open the pipe");
        Pipe { file, path }
    }
}

/// A recording that stops inside a clean filter until it is told to go on. A filter runs
/// while the store holds the lock of recording, which is what the test needs to be in.
struct HeldRecording {
    started: Pipe,
    go: Pipe,
    repo: PathBuf,
}

impl HeldRecording {
    /// Sets up the filter of `*.slow` files in the repository at `repo`.
    fn new(scratch: &Scratch, repo: &Path) -> HeldRecording {
        let started = Pipe::new(scratch.join("started"));
        let go = Pipe::new(scratch.join("go"));
        let once = scratch.join("filter-ran");
        let filter = scratch.join("bin/hold");
        program(
            &filter,
            &format!(
                "if mkdir '{once}' 2>/dev/null; then\n  printf s > '{started}'\n  \
                 read go < '{go}'\nfi\ncat",
                once = once.display(),
                started = started.path.display(),
                go = go.path.display(),
            ),
        );
        let mut configuration = std::fs::OpenOptions::new()
            .append(true)
            .open(repo.join(".git/config"))
            .expect("open the configuration");
        write!(
            configuration,
            "[filter \"hold\"]\n\tclean = '{}'\n\trequired = true\n",
            filter.display()
        )
        .expect("configure the filter");
        write(repo, ".gitattributes", "*.slow filter=hold\n");
        HeldRecording {
            started,
            go,
            repo: repo.to_owned(),
        }
    }

    /// Lets the recording go on. Said once more by the drop, so that a test that failed
    /// does not leave the filter waiting.
    fn release(&mut self) {
        let _ = self.go.file.write_all(b"\n");
    }
}

impl Drop for HeldRecording {
    fn drop(&mut self) {
        self.release();
    }
}

#[test]
fn a_state_is_read_while_another_is_being_recorded() {
    let scratch = Scratch::new();
    let Repository { worktree, .. } = repository(&scratch);
    let (store, list) = listed_store(&scratch);
    let state: RecordedState = store.record(&worktree).expect("record");
    let mut held = HeldRecording::new(&scratch, worktree.root());
    write(&held.repo, "new.slow", "one\n");
    // The reader of the filter's signal waits in a thread of its own, so that the test
    // waits for it with a deadline.
    let mut signal = held.started.file.try_clone().expect("clone the pipe");
    let (told, hears) = mpsc::channel();
    std::thread::spawn(move || {
        let mut byte = [0];
        let _ = told.send(signal.read_exact(&mut byte).map(|()| byte[0]));
    });

    std::thread::scope(|threads| {
        let recording = threads.spawn(|| store.record(&worktree));
        let heard = hears.recv_timeout(DEADLINE).expect("the filter started");
        assert_eq!(heard.expect("a byte"), b's');
        assert!(
            !recording.is_finished(),
            "the recording is held in its filter"
        );

        // The recording holds its own lock; reading is not behind it. The read runs in a
        // thread of its own so that a read that waited for the recording fails the test
        // after the deadline instead of for ever.
        let (answers, answered) = mpsc::channel();
        let (store, state) = (&store, &state);
        let read = threads.spawn(move || {
            let content = store
                .content(state.id.as_str(), Path::new("file-5.txt"))
                .expect("content");
            let _ = answers.send(content);
        });
        let answer = answered.recv_timeout(DEADLINE);
        let still_held = !recording.is_finished();
        held.release();
        read.join().expect("the read did not panic");
        assert_eq!(
            answer.expect("the read answered during the recording"),
            Some(content_of(5, "first"))
        );
        assert!(still_held, "it was still held when the read answered");

        let recorded = recording
            .join()
            .expect("the recording did not panic")
            .expect("the recording");
        assert_ne!(recorded.id, state.id);
        assert_eq!(
            store
                .content(recorded.id.as_str(), Path::new("new.slow"))
                .expect("content"),
            Some(b"one\n".to_vec())
        );
    });
    assert_eq!(content_processes(&list).len(), 1);
}

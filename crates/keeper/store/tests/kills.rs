//! Component tests: a process that types into the Store is killed, and the file it leaves
//! is opened through the Store's calls.
//!
//! The typist is this test binary started again: [`the_typist_types_into_the_store_named_by_its_variable_and_does_nothing_without_it`]
//! does nothing unless the variable `YAAR_STORE_TYPIST` names a database file, and types
//! into that file for ever when it does. A kill test starts it, takes what it reports on
//! its standard output, and kills it with `SIGKILL`.

mod support;

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use support::Scratch;
use yaar_store::{
    Anchor, Author, Comment, CommentId, ReviewId, ReviewRecord, Round, SentBy, Side, Store, Thread,
    ThreadId, WorkspaceId,
};

/// The variable that names the database file the typist types into.
const TYPIST_VARIABLE: &str = "YAAR_STORE_TYPIST";
/// The variable that makes the typist stop typing at `<thread> <length>` and wait for its
/// standard input to end.
const HOLD_VARIABLE: &str = "YAAR_STORE_TYPIST_HOLD";
/// The variable that gives the seed of the kill tests.
const SEED_VARIABLE: &str = "YAAR_STORE_SEED";
/// The name of the test function the typist is: how the test binary is told to run it.
const TYPIST_TEST: &str =
    "the_typist_types_into_the_store_named_by_its_variable_and_does_nothing_without_it";
/// How many characters a comment has when it is sent.
const COMMENT_LEN: usize = 40;
/// How long the parent waits for the typist to report its first saved key: a process
/// starting, a database opening and a first draft saved, on a loaded machine.
const FIRST_SAVE_WITHIN: Duration = Duration::from_secs(60);

/// An id of 32 hexadecimal digits, from a number.
fn id(n: u128) -> String {
    format!("{n:032x}")
}

fn review_id() -> ReviewId {
    ReviewId::new("1".repeat(32))
}

fn workspace() -> WorkspaceId {
    WorkspaceId::new("2".repeat(32))
}

/// The id of thread `n`, from 1.
fn thread_id(n: u128) -> ThreadId {
    ThreadId::new(id(n))
}

/// The id of the one comment of thread `n`: never an id a thread has.
fn comment_id(n: u128) -> CommentId {
    CommentId::new(id((1u128 << 64) + n))
}

/// The number of the thread an id names.
fn number_of(thread: &ThreadId) -> u128 {
    u128::from_str_radix(thread.as_str(), 16).expect("a thread id is hexadecimal")
}

/// What thread `n` is typed from: its number, a space, then letters. Longer than a
/// comment, of which only the first `COMMENT_LEN` characters are typed.
fn text_of(n: u128) -> String {
    let mut text = format!("{n} ");
    text.extend((0..64u128).map(|i| char::from(b'a' + ((n * 7 + i) % 26) as u8)));
    text
}

/// What the comment of thread `n` holds when it is whole.
fn whole_text_of(n: u128) -> String {
    text_of(n).chars().take(COMMENT_LEN).collect()
}

fn round_of(number: u128) -> Round {
    Round {
        number: u32::try_from(number).expect("a round number fits"),
        state: "a".repeat(40),
        sent_by: SentBy::Copied,
    }
}

/// What the typist does: types thread after thread into the file, one character at a
/// time, and sends each thread's comment when it has its 40 characters. It reports on its
/// standard output, one line at a time: `typing <n> <len>` before each key, `saved <n>
/// <len>` after it, `sent <n>` after a send, and `held` when told to wait.
#[test]
fn the_typist_types_into_the_store_named_by_its_variable_and_does_nothing_without_it() {
    let Some(file) = std::env::var_os(TYPIST_VARIABLE) else {
        return;
    };
    // The test harness writes `test <name> ... ` without ending the line.
    println!();
    let hold = std::env::var(HOLD_VARIABLE).ok().map(|hold| {
        let (n, len) = hold.split_once(' ').expect("a hold is `<n> <len>`");
        (
            n.parse::<u128>().expect("a thread number"),
            len.parse::<usize>().expect("a length"),
        )
    });
    let store = Store::open(Path::new(&file)).expect("the typist opens the store");

    let mut review = store.reviews_of(&workspace()).expect("read").pop();
    if review.is_none() {
        store
            .put_review(&ReviewRecord {
                id: review_id(),
                workspace: workspace(),
                agent: None,
                reviewed: None,
                threads: vec![],
                rounds: vec![],
            })
            .expect("put the review");
        review = store.reviews_of(&workspace()).expect("read").pop();
    }
    let review = review.expect("the review is kept");

    // Where the file says to start: the thread, and the length it holds.
    let (mut n, mut from) = match review.threads.last() {
        None => (1, 1),
        Some(last) => {
            let n = number_of(&last.id);
            let comment = &last.comments[0];
            let len = comment.text.chars().count();
            if comment.round.is_some() {
                (n + 1, 1)
            } else if len >= COMMENT_LEN {
                // A kill fell in the send.
                store
                    .put_round(
                        &review_id(),
                        &round_of(n),
                        std::slice::from_ref(&comment.id),
                    )
                    .expect("send the whole draft");
                println!("sent {n}");
                (n + 1, 1)
            } else {
                (n, len + 1)
            }
        }
    };
    loop {
        let text = whole_text_of(n);
        for len in from..=COMMENT_LEN {
            println!("typing {n} {len}");
            let so_far: String = text.chars().take(len).collect();
            if len == 1 {
                store
                    .put_thread(
                        &review_id(),
                        &Thread {
                            id: thread_id(n),
                            anchor: Anchor {
                                tree: "a".repeat(40),
                                path: PathBuf::from("src/lib.rs"),
                                side: Side::New,
                                first: 1,
                                last: 1,
                                text: "let x = 1;".to_owned(),
                            },
                            resolved: false,
                            addressed: false,
                            comments: vec![Comment {
                                id: comment_id(n),
                                author: Author::Reviewer,
                                text: so_far,
                                round: None,
                            }],
                        },
                    )
                    .expect("put the thread");
            } else {
                store
                    .save_draft(&comment_id(n), &so_far)
                    .expect("save the draft");
            }
            println!("saved {n} {len}");
            if hold == Some((n, len)) {
                println!("held");
                // A wait on the parent, which kills this process here.
                let _ = std::io::stdin().read_to_end(&mut Vec::new());
                return;
            }
        }
        store
            .put_round(&review_id(), &round_of(n), &[comment_id(n)])
            .expect("send the comment");
        println!("sent {n}");
        n += 1;
        from = 1;
    }
}

/// A small generator of the kill instants and the cuts: xorshift, so that a seed gives
/// the same draws on every machine.
struct Draws(u64);

impl Draws {
    /// The seed of this run, read from `YAAR_STORE_SEED` when it is set, else from the
    /// clock; printed either way, so that a failure is run again with it.
    fn from_environment() -> Self {
        let seed = std::env::var(SEED_VARIABLE).ok().map_or_else(
            || {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("the clock is after 1970")
                    .as_nanos() as u64
            },
            |seed| seed.parse().expect("the seed is a number"),
        );
        println!("{SEED_VARIABLE}={seed}");
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A number in `0..bound`.
    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

/// The typist, started again from this binary. Dropping it kills the process and waits
/// for it, so that a failing test leaves none.
struct Typist {
    child: Child,
    lines: Receiver<String>,
    reader: Option<JoinHandle<()>>,
    /// Held open, so that the typist's wait on it lasts until it is killed.
    _stdin: Option<ChildStdin>,
}

impl Typist {
    fn start(file: &Path, hold: Option<&str>, stdin_piped: bool) -> Self {
        let mut command = Command::new(std::env::current_exe().expect("the test binary"));
        command
            .args([TYPIST_TEST, "--exact", "--nocapture", "--test-threads=1"])
            .env(TYPIST_VARIABLE, file)
            .env_remove(HOLD_VARIABLE)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .stdin(if stdin_piped {
                Stdio::piped()
            } else {
                Stdio::null()
            });
        if let Some(hold) = hold {
            command.env(HOLD_VARIABLE, hold);
        }
        let mut child = command.spawn().expect("start the typist");
        let stdout = child.stdout.take().expect("piped output");
        let stdin = child.stdin.take();
        let (sender, lines) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            lines,
            reader: Some(reader),
            _stdin: stdin,
        }
    }

    /// Kills the typist, waits until it is gone, and gives every line it printed that was
    /// not taken yet.
    fn kill(&mut self) -> Vec<String> {
        // Killing a process that ended already is not a fault.
        let _ = self.child.kill();
        self.child.wait().expect("wait for the typist");
        if let Some(reader) = self.reader.take() {
            reader.join().expect("the reader ends with the pipe");
        }
        self.lines.try_iter().collect()
    }
}

impl Drop for Typist {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// What the typist reported in one run.
#[derive(Default)]
struct Run {
    /// The last `typing <n> <len>`.
    typing: Option<(u128, usize)>,
    /// The last `saved <n> <len>`.
    saved: Option<(u128, usize)>,
    /// Every `sent <n>`.
    sent: Vec<u128>,
    /// Whether the last line was a `typing` with no `saved` after it.
    mid_key: bool,
    held: bool,
}

impl Run {
    fn take(&mut self, line: &str) {
        let words: Vec<&str> = line.split(' ').collect();
        let number = |at: usize| words.get(at).and_then(|word| word.parse::<u128>().ok());
        match (words[0], number(1), number(2)) {
            ("typing", Some(n), Some(len)) => {
                self.typing = Some((n, len as usize));
                self.mid_key = true;
            }
            ("saved", Some(n), Some(len)) => {
                self.saved = Some((n, len as usize));
                self.mid_key = false;
            }
            ("sent", Some(n), None) => self.sent.push(n),
            ("held", None, None) => self.held = true,
            // The harness's own lines.
            _ => {}
        }
    }
}

/// The one review of the file, read through a store opened on it. Fails when the file
/// does not open or does not hold exactly one review.
fn the_review(file: &Path) -> ReviewRecord {
    let store = Store::open(file).expect("the file opens after the kill");
    let mut reviews = store.reviews_of(&workspace()).expect("the reviews read");
    assert_eq!(reviews.len(), 1, "exactly one review");
    reviews.pop().expect("one review")
}

/// What every file the typist left must satisfy: the threads are 1, 2, 3 and so on with no
/// gap; each comment is a prefix of its text; a thread with a round has the round of its
/// number and a whole comment; only the last thread may be a draft; the rounds are
/// exactly those. Gives the number of threads that were sent.
fn check_shape(review: &ReviewRecord) -> usize {
    let mut sent = 0;
    for (i, thread) in review.threads.iter().enumerate() {
        let n = i as u128 + 1;
        assert_eq!(thread.id, thread_id(n), "the threads follow one another");
        let [comment] = thread.comments.as_slice() else {
            panic!("thread {n} holds one comment");
        };
        assert_eq!(comment.id, comment_id(n));
        assert!(
            whole_text_of(n).starts_with(&comment.text),
            "the comment of thread {n} is a prefix of its text, not {:?}",
            comment.text
        );
        match comment.round {
            Some(round) => {
                assert_eq!(u128::from(round), n, "thread {n} was sent in round {n}");
                assert_eq!(comment.text, whole_text_of(n), "a sent comment is whole");
                sent += 1;
            }
            None => assert_eq!(
                n as usize,
                review.threads.len(),
                "only the last thread may be a draft, and thread {n} is one"
            ),
        }
    }
    let numbers: Vec<u32> = review.rounds.iter().map(|round| round.number).collect();
    let expected: Vec<u32> = (1..=sent as u32).collect();
    assert_eq!(numbers, expected, "the rounds are 1 to the number sent");
    sent
}

/// Three hundred kills while a draft is typed: no review is unreadable, no save that
/// returned is missing, at most one character is lost.
#[test]
fn three_hundred_kills_while_a_draft_is_typed_lose_no_save_and_at_most_one_character() {
    const KILLS: usize = 300;
    let started = Instant::now();
    let mut draws = Draws::from_environment();
    let scratch = Scratch::new();
    let file = scratch.file();
    let mut sent_most = 0u128;
    let mut mid_key = 0;

    for kill in 1..=KILLS {
        let mut typist = Typist::start(&file, None, false);
        let mut run = Run::default();
        // The typist is killed while it types: after its first saved key, and a delay
        // drawn in 0 to 30 ms later.
        while run.saved.is_none() {
            match typist.lines.recv_timeout(FIRST_SAVE_WITHIN) {
                Ok(line) => run.take(&line),
                Err(why) => panic!("kill {kill}: no saved key was reported: {why}"),
            }
        }
        let delay = Duration::from_micros(draws.below(30_000));
        let until = Instant::now() + delay;
        loop {
            match typist
                .lines
                .recv_timeout(until.saturating_duration_since(Instant::now()))
            {
                Ok(line) => run.take(&line),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("kill {kill}: the typist ended by itself")
                }
            }
        }
        for line in typist.kill() {
            run.take(&line);
        }
        drop(typist);
        sent_most = run.sent.iter().copied().fold(sent_most, u128::max);
        mid_key += usize::from(run.mid_key);

        // No review is unreadable.
        let review = the_review(&file);
        // Every comment of every thread is a prefix of its text; only the last may be a
        // draft.
        let sent = check_shape(&review);

        // No save that returned is missing.
        let (n, len) = run.saved.expect("a saved key was reported");
        let thread = review
            .threads
            .iter()
            .find(|thread| number_of(&thread.id) == n)
            .unwrap_or_else(|| panic!("kill {kill}: thread {n} was saved and is not there"));
        assert!(
            thread.comments[0].text.chars().count() >= len,
            "kill {kill}: saved {n} {len}, the file holds {:?}",
            thread.comments[0].text
        );
        // A send the typist reported is in the file. One it was killed in is there or
        // not, and the next run finds it either way.
        assert!(
            sent as u128 >= sent_most,
            "kill {kill}: {sent_most} was reported sent, and {sent} are"
        );

        // At most one character is lost.
        if let Some((n, len)) = run.typing {
            match review.threads.iter().find(|t| number_of(&t.id) == n) {
                Some(thread) => assert!(
                    thread.comments[0].text.chars().count() + 1 >= len,
                    "kill {kill}: typing {n} {len}, the file holds {:?}",
                    thread.comments[0].text
                ),
                None => assert_eq!(len, 1, "kill {kill}: thread {n} is missing at key {len}"),
            }
        }
    }
    println!(
        "{KILLS} kills in {:?}; {mid_key} fell between a typing line and its saved line; \
         {sent_most} comments sent",
        started.elapsed()
    );
}

/// Starts the typist with a hold at thread 3, three characters, and kills it there: two
/// comments are sent, and the write-ahead file holds what was done after the second send.
/// Gives the folder and the lines the typist printed.
fn held_and_killed(attempt: usize) -> (Scratch, Run) {
    let scratch = Scratch::new();
    let mut typist = Typist::start(&scratch.file(), Some("3 3"), true);
    let mut run = Run::default();
    while !run.held {
        match typist.lines.recv_timeout(FIRST_SAVE_WITHIN) {
            Ok(line) => run.take(&line),
            Err(why) => panic!("attempt {attempt}: the typist never held: {why}"),
        }
    }
    for line in typist.kill() {
        run.take(&line);
    }
    assert_eq!(run.sent, [1, 2], "two comments were sent before the hold");
    assert_eq!(run.saved, Some((3, 3)));
    (scratch, run)
}

fn wal_of(scratch: &Scratch) -> PathBuf {
    scratch.0.join("store.db-wal")
}

fn cut(wal: &Path, offset: u64) {
    std::fs::OpenOptions::new()
        .write(true)
        .open(wal)
        .expect("open the write-ahead file")
        .set_len(offset)
        .expect("cut the write-ahead file");
}

/// After a kill at the hold: a folder whose write-ahead file is not empty, started
/// again, at most 3 times. The only way to an empty one is the writer's flush after a
/// pause of a second, which a stalled runner allows.
fn held_with_a_log() -> (Scratch, u64) {
    for attempt in 1..=3 {
        let (scratch, _) = held_and_killed(attempt);
        let length = std::fs::metadata(wal_of(&scratch)).map_or(0, |m| m.len());
        if length > 0 {
            return (scratch, length);
        }
    }
    panic!("the write-ahead file was empty at the hold three times: the machine is stalled");
}

/// What must hold of a file whose write-ahead file was cut: it opens, and the two sent
/// comments are there whole, in rounds 1 and 2, and no more rounds; the third, when it is
/// there, is a draft that is a prefix of its text. Gives the length of the third.
fn check_cut(file: &Path, context: &str) -> Option<usize> {
    let review = the_review(file);
    assert!(
        (2..=3).contains(&review.threads.len()),
        "{context}: threads 1 and 2, and 3 perhaps; found {}",
        review.threads.len()
    );
    assert_eq!(check_shape(&review), 2, "{context}: two comments were sent");
    review
        .threads
        .get(2)
        .map(|thread| thread.comments[0].text.chars().count())
}

/// With the write-ahead file cut at a random place, the database opens and the sent
/// comments are intact.
#[test]
fn with_the_write_ahead_file_cut_at_a_random_place_the_database_opens_and_the_sent_comments_are_intact()
 {
    const TRIALS: usize = 100;
    let mut draws = Draws::from_environment();
    let mut third_kept = 0;
    for trial in 1..=TRIALS {
        let (scratch, length) = held_with_a_log();
        let offset = draws.below(length);
        cut(&wal_of(&scratch), offset);
        // The shared-memory file stays as the kill left it.
        let third = check_cut(&scratch.file(), &format!("trial {trial}, cut at {offset}"));
        third_kept += usize::from(third.is_some());
    }
    println!("{TRIALS} trials; the third thread survived the cut in {third_kept}");
}

/// The cuts that are not left to chance: nothing of the file kept, only its header, and
/// all but its last byte. Before any cut, the log holds the draft that was being typed,
/// so that the cut is what takes it away, and what a send moved into the database file is
/// not taken with it.
#[test]
fn a_write_ahead_file_cut_to_nothing_to_its_header_or_by_a_byte_costs_only_the_draft() {
    let (scratch, length) = held_with_a_log();

    // The uncut file, opened on a copy: the log holds the third thread, three characters.
    let copy = Scratch::new();
    for name in ["store.db", "store.db-wal", "store.db-shm"] {
        let from = scratch.0.join(name);
        if from.exists() {
            std::fs::copy(&from, copy.0.join(name)).expect("copy");
        }
    }
    assert_eq!(check_cut(&copy.file(), "uncut"), Some(3));

    for offset in [0, 32, length - 1] {
        let trial = Scratch::new();
        for name in ["store.db", "store.db-wal", "store.db-shm"] {
            let from = scratch.0.join(name);
            if from.exists() {
                std::fs::copy(&from, trial.0.join(name)).expect("copy");
            }
        }
        cut(&wal_of(&trial), offset);
        let third = check_cut(&trial.file(), &format!("cut at {offset} of {length}"));
        // Only the header, or a log with a frame missing at its end, is a log with no
        // third thread whole: the thread and its two saves each wrote frames.
        if offset <= 32 {
            assert_eq!(third, None, "cut at {offset}: nothing of the log is left");
        } else if let Some(len) = third {
            assert!(len <= 3, "cut at {offset}: the third thread holds {len}");
        }
    }
}

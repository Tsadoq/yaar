//! The clean-up of recorded states, through the engine with the real Store and the real
//! Recorder beneath, on a repository the test makes, with a clock of the test's own: the
//! deletion by the set time and at once, what refers to a state, the history's mark of an
//! expired turn, and the room the stores take.

mod support;

use std::collections::BTreeSet;
use std::os::unix::fs::{MetadataExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use support::{
    Scratch, commit_all, engine, git, id, init, new_thread, open, session, session_in, store,
    workspace, write,
};
use yaar_agent::CopyAsText;
use yaar_recorder::{Clock, CommitId, StateId, Worktree};
use yaar_review_engine::{
    DraftOn, DraftSave, Expired, KEEP_STATES, NewAnchor, Presence, ReviewId, Reviews, Scope,
    SentBy, SessionRecord, Side, StoreRoom, ThreadId, TurnMark,
};
use yaar_store::{CommentId, Store, TurnEndRecord, TurnId, TurnRecord, WorkspaceId};

/// 2026-01-01T00:00:00Z, which the clock of every test starts at.
const BASE: u64 = 1_767_225_600;

const HOUR: Duration = Duration::from_secs(60 * 60);
const DAY: Duration = Duration::from_secs(24 * 60 * 60);
const SECOND: Duration = Duration::from_secs(1);

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

    /// Puts the clock `offset` after its base.
    fn move_to(&self, offset: Duration) {
        *self.moved.lock().expect("the clock") = offset;
    }
}

impl Clock for TestClock {
    fn now(&self) -> SystemTime {
        self.base + *self.moved.lock().expect("the clock")
    }
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

/// A repository with a tracked text file and an untracked `turn.bin`, one commit.
fn repository(scratch: &Scratch) -> PathBuf {
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "notes.txt", "one\ntwo\nthree\n");
    commit_all(&repo, "one");
    std::fs::write(repo.join("turn.bin"), b"first").expect("write turn.bin");
    repo
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
    let common = repo.join(git(repo, &["rev-parse", "--git-common-dir"]));
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

/// A repository with two objects planted in it that no ref reaches, and its reads taken,
/// before the engine runs: what a clean-up must leave as it found it.
struct Watched {
    repo: PathBuf,
    planted: [String; 2],
    before: Reads,
}

/// Plants a blob and a commit that `git stash create` made and left unnamed, then reads
/// the repository. Called after the test's own commits and before anything of the engine
/// runs.
fn watch(scratch: &Scratch, repo: &Path) -> Watched {
    let file = scratch.join("planted.txt");
    std::fs::write(&file, "a blob nothing names\n").expect("write the file");
    let blob = git(repo, &["hash-object", "-w", file.to_str().expect("text")]);
    write(repo, "notes.txt", "edited\n");
    let commit = git(repo, &["stash", "create"]);
    git(repo, &["checkout", "--", "notes.txt"]);
    assert!(!commit.is_empty(), "the stash was created");
    let before = reads(repo);
    Watched {
        repo: repo.to_owned(),
        planted: [blob, commit],
        before,
    }
}

impl Watched {
    /// The reads are as they were, and both planted objects are still there.
    fn assert_unchanged(&self) {
        assert!(
            reads(&self.repo) == self.before,
            "the repository's reads changed"
        );
        for id in &self.planted {
            git(&self.repo, &["cat-file", "-e", id]);
        }
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

/// Whether the store at `folder` holds the object.
fn holds_object(folder: &Path, id: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(folder)
        .args(["cat-file", "-e", id])
        .status()
        .expect("start git")
        .success()
}

/// Every state the engine's store of `workspace` lists, by full id.
fn held(reviews: &Reviews, workspace: &WorkspaceId) -> BTreeSet<String> {
    reviews
        .states_of(workspace)
        .expect("the state store")
        .states()
        .expect("list the states")
        .into_iter()
        .map(|state| state.id.as_str().to_owned())
        .collect()
}

fn set(ids: &[&StateId]) -> BTreeSet<String> {
    ids.iter().map(|id| id.as_str().to_owned()).collect()
}

fn deleted(expired: &Expired) -> BTreeSet<String> {
    expired.states.iter().cloned().collect()
}

/// The room the store of `workspace` takes, as the engine reports it.
fn room_of(reviews: &Reviews, workspace: &WorkspaceId) -> u64 {
    let StoreRoom { workspaces, .. } = reviews.state_sizes().expect("the sizes");
    workspaces
        .into_iter()
        .find(|(one, _)| one == workspace)
        .map(|(_, room)| room)
        .expect("the workspace has a store")
}

fn store_folder(scratch: &Scratch, workspace: &WorkspaceId) -> PathBuf {
    scratch.join(&format!("app/states/{workspace}.git"))
}

/// One workspace with a session and its review, a clock, and the engine over them: the
/// set-up of every test below.
struct Setup {
    scratch: Scratch,
    repo: PathBuf,
    watched: Watched,
    clock: Arc<TestClock>,
    store: Store,
    reviews: Reviews,
    worktree: Worktree,
    session: SessionRecord,
    review: ReviewId,
    noise: Noise,
}

impl Setup {
    fn new() -> Setup {
        let scratch = Scratch::new();
        let repo = repository(&scratch);
        let watched = watch(&scratch, &repo);
        let clock = TestClock::new();
        let store = store(&scratch);
        let reviews = engine(&scratch, store.clone()).with_clock(clock.clone());
        let session = session(&store, 1);
        let review = reviews.session_started(&session).expect("start");
        Setup {
            worktree: open(&repo),
            scratch,
            repo,
            watched,
            clock,
            store,
            reviews,
            session,
            review,
            noise: Noise(0x9e37_79b9_7f4a_7c15),
        }
    }

    /// Writes `turn.bin` anew, so that no state shares its 64 KiB with another.
    fn change_files(&mut self) {
        std::fs::write(self.repo.join("turn.bin"), self.noise.kib(64)).expect("write turn.bin");
    }

    /// A turn of the session ends at the clock's time with `turn.bin` changed: its state is
    /// recorded and the turn kept with it, as the Keeper keeps one.
    fn end_turn(&mut self, number: u32) -> (TurnId, StateId) {
        self.change_files();
        let state = self.end_turn_as_it_is(number);
        (TurnId::new(id(5000 + u128::from(number))), state)
    }

    /// The same with no file changed.
    fn end_turn_as_it_is(&mut self, number: u32) -> StateId {
        let ended = self
            .reviews
            .turn_ended(&self.worktree, &self.session.id, None)
            .expect("the turn ended");
        self.store
            .put_turn(&TurnRecord {
                id: TurnId::new(id(5000 + u128::from(number))),
                session: self.session.id.clone(),
                number,
                ended: Some(TurnEndRecord::Finished),
                start: None,
                state: Some(ended.state.as_str().to_owned()),
            })
            .expect("keep the turn");
        ended.state
    }

    /// A thread on line 2 of `notes.txt` in the files on disk.
    fn thread_on_the_files(&mut self, n: u128) -> String {
        self.change_files();
        self.reviews
            .save_draft(
                Some(&self.worktree),
                &self.review,
                &new_thread(n, "notes.txt", 2, "two", "a thought"),
            )
            .expect("save the draft");
        self.thread_tree(n)
    }

    /// The version thread `n` was written in.
    fn thread_tree(&self, n: u128) -> String {
        self.thread(n).anchor.tree
    }

    fn thread(&self, n: u128) -> yaar_review_engine::Thread {
        let reviews = self
            .store
            .reviews_of(&workspace())
            .expect("read the reviews");
        reviews
            .into_iter()
            .flat_map(|review| review.threads)
            .find(|thread| thread.id == ThreadId::new(id(n)))
            .expect("the thread is kept")
    }

    fn marks(&self) -> std::collections::BTreeMap<TurnId, TurnMark> {
        self.reviews.turn_marks(&workspace()).expect("the marks")
    }

    fn expire(&self, at: Duration, keep_for: Duration, presence: Presence) -> Expired {
        self.clock.move_to(at);
        self.reviews
            .expire(&workspace(), keep_for, presence)
            .expect("expire")
    }

    fn clean_up(&self, at: Duration, presence: Presence) -> Expired {
        self.clock.move_to(at);
        self.reviews
            .clean_up(&workspace(), presence)
            .expect("clean up")
    }

    fn held(&self) -> BTreeSet<String> {
        held(&self.reviews, &workspace())
    }
}

// STATE-09
#[test]
fn after_the_set_time_the_state_of_a_turn_nothing_names_is_deleted_and_the_turn_stays_in_the_history_marked_expired()
 {
    let mut run = Setup::new();
    let w = workspace();

    // At B, turn T1 ends: S1. A new thread on line 2 of the files on disk, with nothing
    // changed, sits on the state recorded within the hour: S1.
    let (t1, s1) = run.end_turn(1);
    run.reviews
        .save_draft(
            Some(&run.worktree),
            &run.review,
            &new_thread(1, "notes.txt", 2, "two", "why this line"),
        )
        .expect("save the draft");
    assert_eq!(run.thread_tree(1), s1.as_str());

    // At B + 1 day, T2 ends: S2, and the round that sends the thread is at S2.
    run.clock.move_to(DAY);
    let (t2, s2) = run.end_turn(2);
    run.reviews
        .send(
            &run.review,
            Scope::Everything,
            SentBy::Copied,
            Some(&s2),
            &mut CopyAsText,
        )
        .expect("send")
        .expect("something was sent");
    let round = run.store.reviews_of(&w).expect("read")[0].rounds[0].clone();
    assert_eq!(round.state.as_deref(), Some(s2.as_str()));

    // At B + 2 days the reviewer marks the change reviewed: Sm.
    run.clock.move_to(2 * DAY);
    run.change_files();
    let sm = run
        .reviews
        .mark_reviewed(&run.worktree, &run.review)
        .expect("mark");
    assert_eq!(
        run.store.reviews_of(&w).expect("read")[0]
            .reviewed
            .as_deref(),
        Some(sm.as_str())
    );

    // At B + 3 days T3 ends: S3, which nothing names. At B + 4 days T4: S4, the newest. The
    // thread has no place of its own: notes.txt never changes, so it sits where its anchor
    // says, in S1.
    run.clock.move_to(3 * DAY);
    let (t3, s3) = run.end_turn(3);
    run.clock.move_to(4 * DAY);
    let (t4, s4) = run.end_turn(4);
    assert_eq!(run.thread(1).now, None);
    assert_eq!(held(&run.reviews, &w), set(&[&s1, &s2, &sm, &s3, &s4]));

    // "After the set time, 30 days with nothing set": one second short of the set time and
    // the hour after S3 was recorded, nothing goes, though S1, S2 and Sm are older.
    assert_eq!(KEEP_STATES, 30 * DAY);
    let short = run.expire(
        3 * DAY + KEEP_STATES + HOUR - SECOND,
        KEEP_STATES,
        Presence::Present,
    );
    assert_eq!(short.states, Vec::<String>::new());
    assert_eq!(held(&run.reviews, &w), set(&[&s1, &s2, &sm, &s3, &s4]));
    let size_before = room_of(&run.reviews, &w);

    // At the set time it deletes S3 and nothing else.
    let expired = run.expire(3 * DAY + KEEP_STATES + HOUR, KEEP_STATES, Presence::Present);
    assert_eq!(expired.workspace, w);
    assert_eq!(expired.states, vec![s3.as_str().to_owned()]);

    // The Recorder deleted that state of a turn no thread, round or mark names: the round's
    // and the mark's are still there.
    assert_eq!(held(&run.reviews, &w), set(&[&s1, &s2, &sm, &s4]));
    assert!(!holds_object(&store_folder(&run.scratch, &w), s3.as_str()));

    // The store is smaller: by the 64 KiB of S3's turn.bin and more than 32 KiB.
    let size_after = room_of(&run.reviews, &w);
    assert!(
        size_before >= size_after + 32 * 1024,
        "the store took {size_before} before and {size_after} after"
    );
    assert_eq!(expired.before, size_before);
    assert_eq!(expired.after, size_after);

    // The turn stays in the history, marked expired.
    let turns = run.store.turns_of(&w).expect("the turns");
    let of_t3 = turns.iter().find(|turn| turn.id == t3).expect("T3 is kept");
    assert_eq!(of_t3.state.as_deref(), Some(s3.as_str()));
    let marks = run.marks();
    assert_eq!(marks.len(), 4);
    assert_eq!(marks[&t3], TurnMark::Expired);
    for held_turn in [&t1, &t2, &t4] {
        assert_eq!(marks[held_turn], TurnMark::Held);
    }

    // A state a thread sits on is still there, and reads.
    assert!(held(&run.reviews, &w).contains(s1.as_str()));
    let content = run
        .reviews
        .states_of(&w)
        .expect("the store")
        .content(s1.as_str(), Path::new("notes.txt"))
        .expect("read");
    assert_eq!(content.as_deref(), Some(&b"one\ntwo\nthree\n"[..]));

    // Nothing of the user's repository was written.
    run.watched.assert_unchanged();
}

// STATE-09 (a turn that changed nothing)
#[test]
fn a_turn_that_changed_nothing_names_a_state_as_young_as_the_turn_and_is_not_taken_with_the_old_one()
 {
    let mut run = Setup::new();
    let w = workspace();
    let (t1, s1) = run.end_turn(1);

    // At B + 40 days a turn ends with nothing changed: a new state, dated that day, with
    // the files of the first. At B + 40 days and an hour a turn ends with turn.bin changed.
    run.clock.move_to(40 * DAY);
    let s2 = run.end_turn_as_it_is(2);
    assert_ne!(s2, s1);
    let (t2, t3) = (TurnId::new(id(5002)), TurnId::new(id(5003)));
    run.clock.move_to(40 * DAY + HOUR);
    let (_, s3) = run.end_turn(3);
    let trees: Vec<String> = run
        .reviews
        .states_of(&w)
        .expect("the store")
        .states()
        .expect("list")
        .into_iter()
        .map(|state| state.tree)
        .collect();
    assert_eq!(
        trees[0], trees[1],
        "the second state holds the first's files"
    );

    let expired = run.expire(41 * DAY, KEEP_STATES, Presence::Present);
    assert_eq!(deleted(&expired), set(&[&s1]));
    let marks = run.marks();
    assert_eq!(marks[&t1], TurnMark::Expired);
    assert_eq!(marks[&t2], TurnMark::Held);
    assert_eq!(marks[&t3], TurnMark::Held);
    assert_eq!(run.held(), set(&[&s2, &s3]));
    run.watched.assert_unchanged();
}

// STATE-09 (a worktree that is gone)
#[test]
fn the_store_of_a_gone_worktree_goes_as_one_once_its_newest_state_is_old_enough_and_opens_anew_empty()
 {
    let mut run = Setup::new();
    let w = workspace();

    // A thread's state recorded at B, the newest at B + 20 days.
    let s_thread = StateId::new(run.thread_on_the_files(1));
    run.clock.move_to(20 * DAY);
    let (_, s_newest) = run.end_turn(1);
    assert_eq!(run.held(), set(&[&s_thread, &s_newest]));
    let folder = store_folder(&run.scratch, &w);

    // At B + 31 days the thread's state is still there: it is older than the set time, but
    // the store goes only once its newest state is.
    let at_31 = run.expire(31 * DAY, KEEP_STATES, Presence::Gone);
    assert_eq!(at_31.states, Vec::<String>::new());
    assert_eq!(run.held(), set(&[&s_thread, &s_newest]));

    // One second short of the set time and the hour after the newest: still there.
    let short = run.expire(
        20 * DAY + KEEP_STATES + HOUR - SECOND,
        KEEP_STATES,
        Presence::Gone,
    );
    assert_eq!(short.states, Vec::<String>::new());
    assert!(folder.is_dir());

    let size_before = room_of(&run.reviews, &w);
    let gone = run.expire(20 * DAY + KEEP_STATES + HOUR, KEEP_STATES, Presence::Gone);
    assert_eq!(
        deleted(&gone),
        set(&[&s_thread, &s_newest]),
        "every state of the store, referred to or not"
    );
    assert_eq!((gone.before, gone.after), (size_before, 0));
    assert!(!folder.exists(), "the store's folder is gone");

    // The thread keeps its own text: a gone worktree's reviews stay.
    assert_eq!(run.thread(1).anchor.tree, s_thread.as_str());

    // The engine opens it anew, empty, and a recording goes into the new folder.
    let fresh = run.reviews.states_of(&w).expect("open anew");
    assert!(folder.is_dir());
    assert!(fresh.states().expect("list").is_empty());
    run.end_turn(2);
    assert_eq!(run.held().len(), 1);
    run.watched.assert_unchanged();
}

#[test]
fn the_clean_up_at_once_deletes_what_nothing_names_but_for_the_hour_and_lets_go_a_kept_commit_nothing_names()
 {
    let mut run = Setup::new();
    let w = workspace();
    let c1 = git(&run.repo, &["rev-parse", "HEAD"]);
    git(&run.repo, &["commit", "--allow-empty", "-q", "-m", "two"]);
    let c2 = git(&run.repo, &["rev-parse", "HEAD"]);
    // The states are taken on a third commit, so that neither of the two is reached
    // through them.
    git(&run.repo, &["commit", "--allow-empty", "-q", "-m", "three"]);
    // The repository is read after its last commit.
    run.watched = watch(&run.scratch, &run.repo);

    // At B: thread A on the files (its anchor is S0), a turn whose state a round names,
    // a mark, a state nothing names (U); a thread written on the commit c1, and c2 kept
    // by nothing's name.
    let s0 = run.thread_on_the_files(1);
    let (_, p1) = run.end_turn(1);
    run.reviews
        .send(
            &run.review,
            Scope::Everything,
            SentBy::Copied,
            Some(&p1),
            &mut CopyAsText,
        )
        .expect("send")
        .expect("something was sent");
    run.change_files();
    let sm = run
        .reviews
        .mark_reviewed(&run.worktree, &run.review)
        .expect("mark");
    let (_, u) = run.end_turn(2);
    let on_commit = DraftSave {
        thread: ThreadId::new(id(2)),
        comment: CommentId::new(id(1002)),
        on: DraftOn::NewThread(NewAnchor {
            tree: Some(c1.clone()),
            path: PathBuf::from("notes.txt"),
            side: Side::New,
            first: 2,
            last: 2,
            text: "two".to_owned(),
        }),
        text: "on a commit".to_owned(),
    };
    run.reviews
        .save_draft(Some(&run.worktree), &run.review, &on_commit)
        .expect("save the draft");
    let states = run.reviews.states_of(&w).expect("the store");
    states
        .keep_commit(&run.worktree, &CommitId::new(&c2))
        .expect("keep c2");
    assert_eq!(states.kept_commits().expect("list"), {
        let mut both = vec![c1.clone(), c2.clone()];
        both.sort();
        both
    });

    // At B + 110 minutes a turn ends (Y), at B + 119 minutes another (Z, the newest): both
    // are younger than the hour at B + 120 minutes.
    run.clock.move_to(110 * 60 * SECOND);
    let (_, y) = run.end_turn(3);
    run.clock.move_to(119 * 60 * SECOND);
    let (_, z) = run.end_turn(4);
    let s0 = StateId::new(s0);

    let expired = run.clean_up(120 * 60 * SECOND, Presence::Present);
    // Only U: S0 is a thread's, p1 a round's, sm a mark's, Y and Z are young.
    assert_eq!(deleted(&expired), set(&[&u]));
    assert_eq!(run.held(), set(&[&s0, &p1, &sm, &y, &z]));
    // The kept commit nothing names is let go, and the one a thread sits on stays.
    assert_eq!(states.kept_commits().expect("list"), vec![c1]);
    assert!(!holds_object(&store_folder(&run.scratch, &w), &c2));
    assert!(expired.after < expired.before);
    run.watched.assert_unchanged();
}

#[test]
fn the_turn_that_is_running_keeps_the_state_its_change_counts_from_until_it_ends() {
    let mut run = Setup::new();
    let (_, start) = run.end_turn(1);
    // Turn 2 is running, counting from `start`; two turns end meanwhile.
    run.store
        .put_turn(&TurnRecord {
            id: TurnId::new(id(5002)),
            session: run.session.id.clone(),
            number: 2,
            ended: None,
            start: Some(start.as_str().to_owned()),
            state: None,
        })
        .expect("keep the running turn");
    let (_, q) = run.end_turn(3);
    run.clock.move_to(DAY);
    let (_, newest) = run.end_turn(4);

    // By the set time: Q goes, `start` is kept though as old.
    let by_time = run.expire(40 * DAY, KEEP_STATES, Presence::Present);
    assert_eq!(deleted(&by_time), set(&[&q]));
    // At once: nothing more goes.
    let at_once = run.clean_up(40 * DAY, Presence::Present);
    assert_eq!(at_once.states, Vec::<String>::new());
    assert_eq!(run.held(), set(&[&start, &newest]));

    // The turn ends: the next clean-up takes `start`.
    run.store
        .end_turn(&TurnId::new(id(5002)), TurnEndRecord::Finished, None)
        .expect("end the turn");
    let after = run.expire(40 * DAY, KEEP_STATES, Presence::Present);
    assert_eq!(deleted(&after), set(&[&start]));
    assert_eq!(run.held(), set(&[&newest]));
    run.watched.assert_unchanged();
}

#[test]
fn the_state_a_thread_was_carried_to_is_kept_with_the_one_it_was_written_in() {
    let mut run = Setup::new();
    // Thread 1 is written in S1, on line 2 of notes.txt.
    let s1 = StateId::new(run.thread_on_the_files(1));
    // A line is added above it: S2, where the thread now sits, on line 3.
    run.clock.move_to(HOUR);
    write(&run.repo, "notes.txt", "zero\none\ntwo\nthree\n");
    let (_, s2) = run.end_turn(1);
    let place = run.thread(1).now.expect("the thread was carried");
    assert_eq!((place.tree.as_str(), place.first), (s2.as_str(), 3));
    // The reviewer resolves it: from then on no state takes it along, and its place stays
    // in S2 while two more states are recorded.
    run.reviews
        .resolve(&run.review, &ThreadId::new(id(1)))
        .expect("resolve");
    run.clock.move_to(DAY);
    let (_, s3) = run.end_turn(2);
    run.clock.move_to(2 * DAY);
    let (_, s4) = run.end_turn(3);
    assert_eq!(run.thread(1).now.expect("a place").tree, s2.as_str());
    write(&run.repo, "notes.txt", "one\ntwo\nthree\n");

    let expired = run.expire(40 * DAY, KEEP_STATES, Presence::Present);
    assert_eq!(
        deleted(&expired),
        set(&[&s3]),
        "S1 is its anchor, S2 its place"
    );
    assert_eq!(run.held(), set(&[&s1, &s2, &s4]));
    run.watched.assert_unchanged();
}

// The newest state of a worktree that is there is a referrer (slice 9 ruling 10). This
// test is the one that shows it, so that it can be taken out alone.
#[test]
fn the_newest_state_of_a_worktree_that_is_there_is_kept_however_old_and_one_of_a_gone_worktree_is_not()
 {
    let mut run = Setup::new();
    let w = workspace();
    let (t1, s0) = run.end_turn(1);
    run.clock.move_to(DAY);
    let (t2, s1) = run.end_turn(2);

    // Forty days on, by the set time: S0 goes, S1 stays as the newest.
    let expired = run.expire(40 * DAY, KEEP_STATES, Presence::Present);
    assert_eq!(deleted(&expired), set(&[&s0]));
    assert_eq!(run.held(), set(&[&s1]));
    // At once, the same: the newest of a worktree that is there stays.
    let at_once = run.clean_up(40 * DAY, Presence::Present);
    assert_eq!(at_once.states, Vec::<String>::new());
    assert_eq!(run.held(), set(&[&s1]));
    let marks = run.marks();
    assert_eq!(
        (marks[&t1], marks[&t2]),
        (TurnMark::Expired, TurnMark::Held)
    );

    // Of a gone worktree it is no referrer: at once it goes, and with it the folder of a
    // store that holds nothing.
    let gone = run.clean_up(40 * DAY, Presence::Gone);
    assert_eq!(deleted(&gone), set(&[&s1]));
    assert_eq!(gone.after, 0);
    assert!(!store_folder(&run.scratch, &w).exists());
    assert_eq!(run.marks()[&t2], TurnMark::Expired);
    run.watched.assert_unchanged();
}

#[test]
fn a_state_from_the_future_is_young_for_both_clean_ups() {
    let mut run = Setup::new();
    run.clock.move_to(10 * DAY);
    let (_, s1) = run.end_turn(1);
    let (_, s2) = run.end_turn(2);

    // The clock goes back before both states were recorded: nothing is old.
    let by_time = run.expire(Duration::ZERO, Duration::ZERO, Presence::Present);
    assert_eq!(by_time.states, Vec::<String>::new());
    let at_once = run.clean_up(Duration::ZERO, Presence::Present);
    assert_eq!(at_once.states, Vec::<String>::new());
    assert_eq!(run.held(), set(&[&s1, &s2]));

    // Control: two hours after they were recorded the first one is old, with no set time.
    let later = run.expire(10 * DAY + 2 * HOUR, Duration::ZERO, Presence::Present);
    assert_eq!(deleted(&later), set(&[&s1]));
    run.watched.assert_unchanged();
}

#[test]
fn a_workspace_with_no_state_store_deletes_nothing_and_makes_none() {
    let run = Setup::new();
    let other = WorkspaceId::new(id(0xcc));
    let folder = store_folder(&run.scratch, &other);
    for presence in [Presence::Present, Presence::Gone] {
        let expired = run
            .reviews
            .expire(&other, KEEP_STATES, presence)
            .expect("expire");
        assert_eq!(
            expired,
            Expired {
                workspace: other.clone(),
                states: vec![],
                before: 0,
                after: 0
            }
        );
        let cleaned = run.reviews.clean_up(&other, presence).expect("clean up");
        assert_eq!(cleaned, expired);
    }
    assert!(run.reviews.turn_marks(&other).expect("marks").is_empty());
    assert!(!folder.exists(), "no store was made");
    assert!(!run.scratch.join("app/states").exists());
}

#[test]
fn a_turn_with_no_state_is_not_recorded_and_the_marks_are_of_the_workspace_alone() {
    let mut run = Setup::new();
    let (_, state) = run.end_turn(1);
    // A turn that ended with no state, and one that is still running.
    run.store
        .put_turn(&TurnRecord {
            id: TurnId::new(id(5002)),
            session: run.session.id.clone(),
            number: 2,
            ended: Some(TurnEndRecord::Interrupted),
            start: None,
            state: None,
        })
        .expect("keep");
    run.store
        .put_turn(&TurnRecord {
            id: TurnId::new(id(5003)),
            session: run.session.id.clone(),
            number: 3,
            ended: None,
            start: Some(state.as_str().to_owned()),
            state: None,
        })
        .expect("keep");
    // A turn of another workspace's session.
    let elsewhere = session_in(&run.store, 2, WorkspaceId::new(id(0xbb)));
    run.store
        .put_turn(&TurnRecord {
            id: TurnId::new(id(5009)),
            session: elsewhere.id.clone(),
            number: 1,
            ended: Some(TurnEndRecord::Finished),
            start: None,
            state: Some("f".repeat(40)),
        })
        .expect("keep");

    let marks = run.marks();
    assert_eq!(
        marks.into_iter().collect::<Vec<_>>(),
        vec![
            (TurnId::new(id(5001)), TurnMark::Held),
            (TurnId::new(id(5002)), TurnMark::NotRecorded),
            (TurnId::new(id(5003)), TurnMark::NotRecorded),
        ]
    );
}

#[test]
fn the_room_is_each_stores_blocks_by_workspace_and_a_folder_of_another_name_counts_in_the_total_only()
 {
    let mut run = Setup::new();
    let first = workspace();
    let second = WorkspaceId::new(id(0xbb));
    run.end_turn(1);
    let started = session_in(&run.store, 2, second.clone());
    run.reviews.session_started(&started).expect("start");
    run.change_files();
    run.reviews
        .turn_ended(&run.worktree, &started.id, None)
        .expect("a turn of the second workspace");

    let states = run.scratch.join("app/states");
    write(&states, "junk/leftover.bin", &"x".repeat(20_000));
    write(&states, "note.txt", "a plain file");
    let room = run.reviews.state_sizes().expect("the sizes");
    assert_eq!(
        room.workspaces,
        vec![
            (
                first.clone(),
                blocks_of(&store_folder(&run.scratch, &first))
            ),
            (
                second.clone(),
                blocks_of(&store_folder(&run.scratch, &second))
            ),
        ]
    );
    assert_eq!(room.total, blocks_of(&states));
    let sum: u64 = room.workspaces.iter().map(|(_, room)| room).sum();
    assert!(
        room.total >= sum + blocks_of(&states.join("junk")),
        "the junk folder is in the total"
    );
    run.watched.assert_unchanged();
}

#[test]
fn with_no_folder_of_states_the_room_is_nothing() {
    let run = Setup::new();
    assert_eq!(
        run.reviews.state_sizes().expect("the sizes"),
        StoreRoom {
            workspaces: vec![],
            total: 0
        }
    );
}

#[test]
fn a_store_that_is_a_link_is_neither_cleaned_nor_removed_and_its_target_is_as_before() {
    let mut run = Setup::new();
    let w = workspace();
    let (_, s0) = run.end_turn(1);
    run.clock.move_to(DAY);
    let (_, s1) = run.end_turn(2);
    let folder = store_folder(&run.scratch, &w);
    // The user moved the store to another disk and left a link.
    let target = run.scratch.join("elsewhere/states.git");
    std::fs::create_dir_all(target.parent().expect("a folder")).expect("make the folder");
    let engine_with_the_store_open = std::mem::replace(
        &mut run.reviews,
        engine(&run.scratch, run.store.clone()).with_clock(run.clock.clone()),
    );
    drop(engine_with_the_store_open);
    std::fs::rename(&folder, &target).expect("move the store");
    symlink(&target, &folder).expect("link it");
    let files_before = files_below(&target);
    let refs_before = git(&target, &["for-each-ref"]);

    for presence in [Presence::Present, Presence::Gone] {
        let by_time = run.expire(400 * DAY, KEEP_STATES, presence);
        assert_eq!(by_time.states, Vec::<String>::new());
        let at_once = run.clean_up(400 * DAY, presence);
        assert_eq!(at_once.states, Vec::<String>::new());
        assert!(
            std::fs::symlink_metadata(&folder)
                .expect("the link")
                .is_symlink()
        );
        assert_eq!(files_below(&target), files_before);
        assert_eq!(git(&target, &["for-each-ref"]), refs_before);
    }
    // It is listed by its name, counted as the link.
    let room = run.reviews.state_sizes().expect("the sizes");
    assert_eq!(
        room.workspaces,
        vec![(w.clone(), blocks_of(&folder))],
        "the link's own blocks"
    );
    assert_eq!(
        states_in(&target),
        set(&[&s0, &s1]),
        "both states are in the target"
    );
    run.watched.assert_unchanged();
}

// The states of the store at `folder`, read by git directly.
fn states_in(folder: &Path) -> BTreeSet<String> {
    git(
        folder,
        &[
            "for-each-ref",
            "--format=%(objectname)",
            "refs/yaar/states/",
        ],
    )
    .lines()
    .map(str::to_owned)
    .collect()
}

//! Following threads through the states the engine records and between any two trees,
//! through the engine with the real Store and the real Recorder beneath, on repositories
//! the test makes.

mod support;

use std::path::{Path, PathBuf};

use support::{
    Scratch, commit_all, engine, git, id, init, new_thread, open, store, workspace, write,
};
use yaar_recorder::{CommitId, Worktree};
use yaar_review_engine::{
    Anchor, Author, Comment, CommentId, DraftOn, DraftSave, EngineError, FileStep, NewAnchor,
    Notice, NoticeRecord, Place, Placed, ReviewId, Reviews, Side, Thread, ThreadId, ThreadPlace,
    WorkspaceId, carry,
};
use yaar_store::Store;

/// A file of `n` lines, `line 1` to `line n`.
fn numbered(n: u32) -> Vec<String> {
    (1..=n).map(|i| format!("line {i}")).collect()
}

fn text(lines: &[String]) -> String {
    lines.iter().map(|line| format!("{line}\n")).collect()
}

/// The line that moves in the cases below: more than twenty letters and digits.
const LONG: &str = "let total_value = compute(first, second);";

/// One case of the thread tests of the follower (THREAD-01 to THREAD-05 and THREAD-07), with
/// the contents, the lines and the answer those tests give, as a table: what the engine
/// must give whatever kind of tree the two versions are.
struct Case {
    name: &'static str,
    before: &'static str,
    after: &'static str,
    old: String,
    new: String,
    /// The lines the thread sits on in the old version.
    lines: (u32, u32),
    /// Where it sits in the new one, and its notice.
    expected: (u32, u32, Option<Notice>),
}

fn cases() -> Vec<Case> {
    let case = |name, old: &[String], new: &[String], lines, expected| Case {
        name,
        before: "a.rs",
        after: "a.rs",
        old: text(old),
        new: text(new),
        lines,
        expected,
    };
    let mut cases = Vec::new();

    // THREAD-01: unchanged lines below an insertion.
    let mut new = vec![
        "new 1".to_string(),
        "new 2".to_string(),
        "new 3".to_string(),
    ];
    new.extend(numbered(10));
    cases.push(case(
        "unchanged lines below an insertion",
        &numbered(10),
        &new,
        (6, 7),
        (9, 10, None),
    ));

    // THREAD-02: a block of twenty letters and digits moved within the file.
    let mut old = numbered(20);
    old[2] = LONG.to_string();
    let mut new = numbered(20);
    new.remove(2);
    new.insert(14, LONG.to_string());
    cases.push(case(
        "a block of twenty letters and digits moved",
        &old,
        &new,
        (3, 3),
        (15, 15, None),
    ));

    // THREAD-03: a rewritten line.
    let mut new = numbered(10);
    new.splice(
        4..5,
        ["first replacement", "second replacement"].map(str::to_string),
    );
    cases.push(case(
        "a rewritten line",
        &numbered(10),
        &new,
        (5, 5),
        (5, 6, Some(Notice::Changed)),
    ));

    // THREAD-04: a deleted line.
    let mut new = numbered(8);
    new.drain(3..5);
    cases.push(case(
        "deleted lines",
        &numbered(8),
        &new,
        (4, 5),
        (4, 4, Some(Notice::Deleted)),
    ));

    // THREAD-05: a line inserted inside a commented range.
    let mut new = numbered(8);
    new.insert(4, "inserted".to_string());
    cases.push(case(
        "a line inserted inside a commented range",
        &numbered(8),
        &new,
        (3, 6),
        (3, 7, Some(Notice::Changed)),
    ));

    // THREAD-07: a file renamed with small edits.
    let mut new = numbered(12);
    new[10] = "edited far below".to_string();
    new.insert(2, "inserted above".to_string());
    cases.push(Case {
        before: "src/old name.rs",
        after: "src/new name.rs",
        ..case(
            "a file renamed with small edits",
            &numbered(12),
            &new,
            (5, 6),
            (6, 7, None),
        )
    });
    cases
}

/// The thread of a case, as it sits in the old version.
fn placed_in_the_old_version(case: &Case) -> Placed {
    Placed {
        at: Place {
            path: PathBuf::from(case.before),
            side: Side::New,
            first: case.lines.0,
            last: case.lines.1,
        },
        notice: None,
    }
}

/// The thread of a case, as it must sit in the new version.
fn placed_in_the_new_version(case: &Case) -> Placed {
    let (first, last, notice) = case.expected;
    Placed {
        at: Place {
            path: PathBuf::from(case.after),
            side: Side::New,
            first,
            last,
        },
        notice,
    }
}

/// The follower's own answer for a case, on the bytes of the two versions: what the other
/// ways of reading the trees must equal.
fn carried_on_the_bytes(case: &Case) -> Placed {
    let (old, new) = (case.old.as_bytes(), case.new.as_bytes());
    let step = if case.before == case.after {
        FileStep::Changed {
            before: old,
            after: new,
        }
    } else {
        FileStep::Renamed {
            to: Path::new(case.after),
            before: old,
            after: new,
        }
    };
    carry(&placed_in_the_old_version(case), Side::New, step)
}

#[test]
fn the_table_of_cases_is_what_the_follower_gives_on_the_bytes() {
    for case in cases() {
        assert_eq!(
            carried_on_the_bytes(&case),
            placed_in_the_new_version(&case),
            "{}",
            case.name
        );
    }
}

/// The repository of one run of a case, with its worktree, and the engine's way to its
/// state store.
struct Run<'a> {
    reviews: &'a Reviews,
    workspace: WorkspaceId,
    repo: PathBuf,
    worktree: Worktree,
}

impl<'a> Run<'a> {
    /// A repository of its own in the scratch folder, as workspace number `n`.
    fn new(scratch: &Scratch, reviews: &'a Reviews, n: u128) -> Run<'a> {
        let repo = scratch.join(&format!("repo{n}"));
        init(&repo);
        Run {
            reviews,
            workspace: WorkspaceId::new(id(0x1000 + n)),
            worktree: open(&repo),
            repo,
        }
    }

    /// Makes the files of the worktree what `files` say: each path with its content; no
    /// other file stays.
    fn set(&self, files: &[(&str, &str)]) {
        for entry in std::fs::read_dir(&self.repo).expect("read the folder") {
            let path = entry.expect("an entry").path();
            if path.file_name().is_some_and(|name| name == ".git") {
                continue;
            }
            if path.is_dir() {
                std::fs::remove_dir_all(&path).expect("remove a folder");
            } else {
                std::fs::remove_file(&path).expect("remove a file");
            }
        }
        for (path, content) in files {
            write(&self.repo, path, content);
        }
    }

    /// Commits what the worktree holds, and has the state store keep the commit.
    fn commit(&self, message: &str) -> String {
        let commit = commit_all(&self.repo, message);
        self.reviews
            .states_of(&self.workspace)
            .expect("the state store")
            .keep_commit(&self.worktree, &CommitId::new(&commit))
            .expect("keep the commit");
        commit
    }

    /// Records the state of what the worktree holds.
    fn record(&self) -> String {
        self.reviews
            .states_of(&self.workspace)
            .expect("the state store")
            .record(&self.worktree)
            .expect("record")
            .id
            .as_str()
            .to_owned()
    }

    /// Where a thread placed as `placed` in tree `from` sits in tree `to`.
    fn carry(&self, placed: &Placed, from: &str, to: &str) -> Placed {
        self.reviews
            .carry_between(&self.workspace, placed, from, to)
            .expect("carry between the trees")
    }
}

/// Runs every case with `way`, which puts the two versions of the case in trees of one
/// kind or another in the repository and gives where the thread sits in the second.
fn every_case_gives_its_placement(way: impl Fn(&Run<'_>, &Case) -> Placed) {
    let scratch = Scratch::new();
    let reviews = engine(&scratch, store(&scratch));
    for (n, case) in cases().iter().enumerate() {
        let run = Run::new(&scratch, &reviews, n as u128);
        assert_eq!(
            way(&run, case),
            placed_in_the_new_version(case),
            "{}",
            case.name
        );
    }
}

// REV-23
#[test]
fn the_thread_cases_give_the_same_placement_between_two_commits() {
    every_case_gives_its_placement(|run, case| {
        run.set(&[(case.before, &case.old)]);
        let first = run.commit("first");
        run.set(&[(case.after, &case.new)]);
        let last = run.commit("last");
        run.carry(&placed_in_the_old_version(case), &first, &last)
    });
}

// REV-23
#[test]
fn the_thread_cases_give_the_same_placement_between_two_recorded_states() {
    every_case_gives_its_placement(|run, case| {
        run.set(&[(case.before, &case.old)]);
        let first = run.record();
        run.set(&[(case.after, &case.new)]);
        let last = run.record();
        assert_ne!(first, last, "{}: two states", case.name);
        run.carry(&placed_in_the_old_version(case), &first, &last)
    });
}

// REV-23
#[test]
fn the_thread_cases_give_the_same_placement_from_a_commit_to_a_recorded_state() {
    every_case_gives_its_placement(|run, case| {
        run.set(&[(case.before, &case.old)]);
        let commit = run.commit("first");
        run.set(&[(case.after, &case.new)]);
        let state = run.record();
        run.carry(&placed_in_the_old_version(case), &commit, &state)
    });
}

// REV-23
#[test]
fn the_thread_cases_give_the_same_placement_over_a_range_of_commits_cut_in_two_either_way() {
    // The change of a case is cut from a line added at the end of the file, which no
    // thread of the cases sits near: the change comes first or last, the other half is
    // the added line. The first version is the commit `A`, the last `C`, `B` between.
    for change_first in [false, true] {
        every_case_gives_its_placement(|run, case| {
            let tail = |content: &str| format!("{content}added at the end\n");
            run.set(&[(case.before, &case.old)]);
            let a = run.commit("A");
            if change_first {
                run.set(&[(case.after, &case.new)]);
            } else {
                run.set(&[(case.before, &tail(&case.old))]);
            }
            let b = run.commit("B");
            run.set(&[(case.after, &tail(&case.new))]);
            let c = run.commit("C");

            let at_b = run.carry(&placed_in_the_old_version(case), &a, &b);
            run.carry(&at_b, &b, &c)
        });
    }
}

/// Reads one thread of a review as a second engine over the Store reads it.
fn thread_of(reviews: &Reviews, review: &ReviewId, thread: u128) -> Thread {
    reviews
        .review(review)
        .expect("read the review")
        .thread(&ThreadId::new(id(thread)))
        .expect("the thread is kept")
        .clone()
}

/// What the tests of the engine share: a repository whose file `a.txt` holds the lines
/// `line 1` to `line n`, the review of the workspace, and the engine.
struct World {
    scratch: Scratch,
    store: Store,
    reviews: Reviews,
    repo: PathBuf,
    worktree: Worktree,
    review: ReviewId,
}

impl World {
    fn new(lines: u32) -> World {
        let scratch = Scratch::new();
        let repo = scratch.join("repo");
        init(&repo);
        write(&repo, "a.txt", &text(&numbered(lines)));
        commit_all(&repo, "first");
        let store = store(&scratch);
        let reviews = engine(&scratch, store.clone());
        let worktree = open(&repo);
        let review = reviews
            .of_workspace(&workspace(), Some(&worktree))
            .expect("read")
            .reviews[0]
            .id()
            .clone();
        World {
            scratch,
            store,
            reviews,
            repo,
            worktree,
            review,
        }
    }

    /// Starts thread `n` on a line of `a.txt` as it is on disk now.
    fn thread_on(&self, n: u128, line: u32) {
        let save = new_thread(n, "a.txt", line, &format!("line {line}"), "a comment");
        self.reviews
            .save_draft(Some(&self.worktree), &self.review, &save)
            .expect("save the draft");
    }

    /// Puts the file `a.txt` as `lines` say.
    fn rewrite(&self, lines: &[String]) {
        write(&self.repo, "a.txt", &text(lines));
    }

    /// Records the state of the files as they are, with no following.
    fn record(&self) -> yaar_recorder::StateId {
        self.reviews
            .states_of(&workspace())
            .expect("the state store")
            .record(&self.worktree)
            .expect("record")
            .id
    }

    /// A second engine over the same Store, which knows nothing the first one holds.
    fn second_engine(&self) -> Reviews {
        engine(&self.scratch, self.store.clone())
    }
}

/// `line 1` to `line n` with `inserted` put before line `at` (counted from 1).
fn with_a_line_before(n: u32, at: usize, inserted: &str) -> Vec<String> {
    let mut lines = numbered(n);
    lines.insert(at - 1, inserted.to_string());
    lines
}

fn place(tree: &yaar_recorder::StateId, path: &str, first: u32, last: u32) -> ThreadPlace {
    ThreadPlace {
        tree: tree.as_str().to_owned(),
        path: PathBuf::from(path),
        side: Side::New,
        first,
        last,
        notice: None,
    }
}

// THREAD-03, the half the follower's own test cannot show: what the engine keeps.
#[test]
fn a_thread_on_a_rewritten_line_is_kept_with_its_notice_and_the_line_it_was_written_on() {
    let world = World::new(10);
    world.thread_on(1, 5);
    let anchor_tree = world
        .reviews
        .review(&world.review)
        .expect("read")
        .thread(&ThreadId::new(id(1)))
        .expect("the thread")
        .anchor
        .tree
        .clone();
    let mut lines = numbered(10);
    lines.splice(
        4..5,
        ["first replacement", "second replacement"].map(str::to_string),
    );
    world.rewrite(&lines);
    let rewritten = world.record();

    let changed = world
        .reviews
        .follow(&workspace(), &rewritten)
        .expect("follow");
    assert_eq!(changed, vec![ThreadId::new(id(1))]);

    // A second engine over the Store reads it back.
    let thread = thread_of(&world.second_engine(), &world.review, 1);
    assert_eq!(
        thread.now,
        Some(ThreadPlace {
            notice: Some(NoticeRecord::Changed),
            ..place(&rewritten, "a.txt", 5, 6)
        })
    );
    assert_eq!(
        thread.anchor,
        Anchor {
            tree: anchor_tree,
            path: PathBuf::from("a.txt"),
            side: Side::New,
            first: 5,
            last: 5,
            text: "line 5".to_string(),
        },
        "the thread keeps the line it was written on, where and as it was written"
    );
}

#[test]
fn a_state_followed_after_a_newer_one_leaves_the_threads_in_the_newer() {
    let world = World::new(10);
    world.thread_on(1, 5);
    // The two states are recorded in this order and followed in the other: the calls of
    // two recordings that ran at once.
    world.rewrite(&with_a_line_before(10, 1, "one above"));
    let older = world.record();
    let mut lines = with_a_line_before(10, 1, "one above");
    lines.insert(0, "and another".to_string());
    world.rewrite(&lines);
    let newer = world.record();

    let first = world
        .reviews
        .follow(&workspace(), &newer)
        .expect("follow the newer");
    let second = world
        .reviews
        .follow(&workspace(), &older)
        .expect("follow the older");

    assert_eq!(first, vec![ThreadId::new(id(1))]);
    assert_eq!(second, vec![], "nothing is carried back");
    let thread = thread_of(&world.second_engine(), &world.review, 1);
    assert_eq!(thread.now, Some(place(&newer, "a.txt", 7, 7)));
}

#[test]
fn a_hundred_threads_on_one_file_are_all_placed_by_one_follow() {
    let world = World::new(100);
    let first = world.record();
    for n in 1..=100u32 {
        let thread = Thread {
            id: ThreadId::new(id(u128::from(n))),
            anchor: Anchor {
                tree: first.as_str().to_owned(),
                path: PathBuf::from("a.txt"),
                side: Side::New,
                first: n,
                last: n,
                text: format!("line {n}"),
            },
            resolved: false,
            addressed: false,
            comments: vec![Comment {
                id: CommentId::new(id(u128::from(n) + 1000)),
                author: Author::Reviewer,
                text: format!("comment {n}"),
                round: None,
            }],
            now: None,
        };
        world
            .store
            .put_thread(&world.review, &thread)
            .expect("keep the thread");
    }
    world.rewrite(&with_a_line_before(100, 1, "at the top"));
    let moved = world.record();

    let changed = world.reviews.follow(&workspace(), &moved).expect("follow");

    assert_eq!(changed.len(), 100);
    let reviews = world.second_engine();
    for n in 1..=100u32 {
        let thread = thread_of(&reviews, &world.review, u128::from(n));
        assert_eq!(
            thread.now,
            Some(place(&moved, "a.txt", n + 1, n + 1)),
            "thread {n}"
        );
    }
}

#[test]
fn a_resolved_thread_and_a_thread_on_the_old_side_stay_and_a_reopened_one_catches_up() {
    let world = World::new(10);
    let first = world.record();
    world.thread_on(1, 5);
    world.thread_on(2, 6);
    // A thread on the old side of the diff: the engine does not move it.
    let old_side = Thread {
        id: ThreadId::new(id(3)),
        anchor: Anchor {
            tree: first.as_str().to_owned(),
            path: PathBuf::from("a.txt"),
            side: Side::Old,
            first: 7,
            last: 7,
            text: "line 7".to_string(),
        },
        resolved: false,
        addressed: false,
        comments: vec![Comment {
            id: CommentId::new(id(1003)),
            author: Author::Reviewer,
            text: "on the old side".to_string(),
            round: None,
        }],
        now: None,
    };
    world
        .store
        .put_thread(&world.review, &old_side)
        .expect("keep");
    world
        .store
        .set_resolved(&ThreadId::new(id(1)), true)
        .expect("resolve");

    world.rewrite(&with_a_line_before(10, 1, "one above"));
    let second = world.record();
    let changed = world.reviews.follow(&workspace(), &second).expect("follow");

    assert_eq!(changed, vec![ThreadId::new(id(2))]);
    let reviews = world.second_engine();
    assert_eq!(thread_of(&reviews, &world.review, 1).now, None);
    assert_eq!(thread_of(&reviews, &world.review, 3).now, None);
    assert_eq!(
        thread_of(&reviews, &world.review, 2).now,
        Some(place(&second, "a.txt", 7, 7))
    );

    // Reopened after two more states, the thread is carried from where it sat across
    // both at once.
    world
        .store
        .set_resolved(&ThreadId::new(id(1)), false)
        .expect("reopen");
    let mut lines = with_a_line_before(10, 1, "one above");
    lines.insert(0, "and another".to_string());
    world.rewrite(&lines);
    let third = world.record();
    world.reviews.follow(&workspace(), &third).expect("follow");
    assert_eq!(
        thread_of(&world.second_engine(), &world.review, 1).now,
        Some(place(&third, "a.txt", 7, 7))
    );
}

#[test]
fn a_state_the_engine_records_carries_the_threads_to_it_and_leaves_alone_a_thread_that_did_not_move()
 {
    let world = World::new(10);
    world.thread_on(1, 2);
    world.thread_on(2, 8);
    world.rewrite(&with_a_line_before(10, 5, "inserted"));

    let state = world
        .reviews
        .mark_reviewed(&world.worktree, &world.review)
        .expect("mark reviewed");

    let reviews = world.second_engine();
    assert_eq!(
        thread_of(&reviews, &world.review, 1).now,
        None,
        "a thread above the insertion sits where its anchor says"
    );
    assert_eq!(
        thread_of(&reviews, &world.review, 2).now,
        Some(place(&state, "a.txt", 9, 9))
    );
}

#[test]
fn a_thread_that_cannot_be_followed_does_not_fail_the_recording_nor_stop_the_others() {
    let world = World::new(10);
    world.thread_on(1, 8);
    // A thread on a commit the state store never held: nothing can be carried from it.
    let unheld = "1".repeat(40);
    let save = DraftSave {
        thread: ThreadId::new(id(2)),
        comment: CommentId::new(id(1002)),
        on: DraftOn::NewThread(NewAnchor {
            tree: Some(unheld),
            path: PathBuf::from("a.txt"),
            side: Side::New,
            first: 3,
            last: 3,
            text: "line 3".to_string(),
        }),
        text: "on a commit nobody kept".to_string(),
    };
    world
        .reviews
        .save_draft(None, &world.review, &save)
        .expect("save the draft");
    world.rewrite(&with_a_line_before(10, 1, "one above"));

    let state = world
        .reviews
        .mark_reviewed(&world.worktree, &world.review)
        .expect("the recording succeeds");

    let reviews = world.second_engine();
    assert_eq!(
        thread_of(&reviews, &world.review, 1).now,
        Some(place(&state, "a.txt", 9, 9)),
        "the thread that could be followed was"
    );
    assert_eq!(thread_of(&reviews, &world.review, 2).now, None);
    // Asked directly, the failure is given.
    assert!(matches!(
        world.reviews.follow(&workspace(), &state),
        Err(EngineError::Git(_))
    ));
}

#[test]
fn a_thread_written_on_a_commit_is_carried_from_it_to_a_recorded_state() {
    let world = World::new(10);
    let commit = git(&world.repo, &["rev-parse", "HEAD"]);
    let save = DraftSave {
        thread: ThreadId::new(id(1)),
        comment: CommentId::new(id(1001)),
        on: DraftOn::NewThread(NewAnchor {
            tree: Some(commit.clone()),
            path: PathBuf::from("a.txt"),
            side: Side::New,
            first: 5,
            last: 5,
            text: "line 5".to_string(),
        }),
        text: "on the commit".to_string(),
    };
    world
        .reviews
        .save_draft(Some(&world.worktree), &world.review, &save)
        .expect("save the draft");
    world.rewrite(&with_a_line_before(10, 2, "inserted"));
    let state = world.record();

    world.reviews.follow(&workspace(), &state).expect("follow");

    let thread = thread_of(&world.second_engine(), &world.review, 1);
    assert_eq!(thread.now, Some(place(&state, "a.txt", 6, 6)));
    assert_eq!(thread.anchor.tree, commit);
}

#[test]
fn a_thread_follows_its_file_to_a_new_name_and_says_when_the_file_is_gone() {
    let world = World::new(10);
    world.thread_on(1, 5);
    world.thread_on(2, 9);
    write(&world.repo, "b.txt", &text(&numbered(10)));
    commit_all(&world.repo, "second file");
    let both = world.record();
    let save = new_thread(3, "b.txt", 4, "line 4", "on the file that goes");
    world
        .reviews
        .save_draft(Some(&world.worktree), &world.review, &save)
        .expect("save the draft");

    // `a.txt` is renamed with a small edit, `b.txt` is deleted.
    std::fs::remove_file(world.repo.join("a.txt")).expect("remove");
    std::fs::remove_file(world.repo.join("b.txt")).expect("remove");
    let mut renamed = numbered(10);
    renamed[0] = "edited".to_string();
    write(&world.repo, "c.txt", &text(&renamed));
    let state = world.record();
    assert_ne!(both, state);

    world.reviews.follow(&workspace(), &state).expect("follow");

    let reviews = world.second_engine();
    assert_eq!(
        thread_of(&reviews, &world.review, 1).now,
        Some(place(&state, "c.txt", 5, 5))
    );
    assert_eq!(
        thread_of(&reviews, &world.review, 3).now,
        Some(ThreadPlace {
            notice: Some(NoticeRecord::Deleted),
            ..place(&state, "b.txt", 4, 4)
        })
    );
}

#[test]
fn a_state_the_store_does_not_hold_carries_nothing_and_a_thread_on_the_old_side_is_not_read_for() {
    let world = World::new(10);
    world.thread_on(1, 5);
    world.rewrite(&with_a_line_before(10, 1, "one above"));

    let unknown = yaar_recorder::StateId::new("2".repeat(40));
    assert_eq!(
        world
            .reviews
            .follow(&workspace(), &unknown)
            .expect("follow"),
        vec![]
    );
    assert_eq!(thread_of(&world.reviews, &world.review, 1).now, None);

    // Trees that do not exist tell a thread on the old side nothing: it stays.
    let on_the_old_side = Placed {
        at: Place {
            path: PathBuf::from("a.txt"),
            side: Side::Old,
            first: 4,
            last: 4,
        },
        notice: None,
    };
    let got = world
        .reviews
        .carry_between(
            &workspace(),
            &on_the_old_side,
            &"3".repeat(40),
            &"4".repeat(40),
        )
        .expect("carry");
    assert_eq!(got, on_the_old_side);
}

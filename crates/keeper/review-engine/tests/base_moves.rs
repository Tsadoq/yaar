//! Threads on the old side when the commit a workspace's changes are counted from moves,
//! through the engine with the real Store and the real Recorder beneath, on a repository
//! the test makes.

mod support;

use std::path::PathBuf;

use support::{
    Scratch, commit_all, engine, git, id, init, new_thread, open, store, workspace, write,
};
use yaar_recorder::{CommitId, Worktree};
use yaar_review_engine::{
    Anchor, Author, Comment, CommentId, DraftOn, DraftSave, EngineError, NewAnchor, NoticeRecord,
    ReviewId, Reviews, Side, Thread, ThreadId, ThreadPlace,
};
use yaar_store::Store;

/// `line 1` to `line n`, one to a line.
fn numbered(n: u32) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

/// What the tests of this file share: a repository with `main` and a branch `b`, the
/// worktree on `b`, the review of the workspace and the engine.
struct World {
    scratch: Scratch,
    store: Store,
    reviews: Reviews,
    repo: PathBuf,
    worktree: Worktree,
    review: ReviewId,
    /// `main` before it moves.
    m1: CommitId,
}

impl World {
    /// `f` holds `line 1` to `line 6` on `main` at `m1`; the branch `b` is checked out.
    fn new() -> World {
        let scratch = Scratch::new();
        let repo = scratch.join("repo");
        init(&repo);
        write(&repo, "f", &numbered(6));
        let m1 = CommitId::new(commit_all(&repo, "first"));
        git(&repo, &["checkout", "--quiet", "-b", "b"]);
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
            m1,
        }
    }

    /// Puts `main` on a new commit whose files are `files`, with `b` checked out again,
    /// and gives the commit.
    fn move_main(&self, files: &[(&str, Option<&str>)]) -> CommitId {
        git(&self.repo, &["checkout", "--quiet", "main"]);
        for (path, content) in files {
            match content {
                Some(content) => write(&self.repo, path, content),
                None => std::fs::remove_file(self.repo.join(path)).expect("remove"),
            }
        }
        let moved = CommitId::new(commit_all(&self.repo, "main moves"));
        git(&self.repo, &["checkout", "--quiet", "b"]);
        moved
    }

    /// Writes thread `n` on `line` of `path` on the old side of `commit`, as a reviewer
    /// does on a removed line of a change counted from it.
    fn old_thread(&self, n: u128, commit: &CommitId, path: &str, line: u32, text: &str) {
        let save = DraftSave {
            thread: ThreadId::new(id(n)),
            comment: CommentId::new(id(n + 1000)),
            on: DraftOn::NewThread(NewAnchor {
                tree: Some(commit.as_str().to_owned()),
                path: PathBuf::from(path),
                side: Side::Old,
                first: line,
                last: line,
                text: text.to_owned(),
            }),
            text: "a comment".to_owned(),
        };
        self.reviews
            .save_draft(Some(&self.worktree), &self.review, &save)
            .expect("save the draft");
    }

    fn thread(&self, n: u128) -> Thread {
        self.second_engine()
            .review(&self.review)
            .expect("read the review")
            .thread(&ThreadId::new(id(n)))
            .expect("the thread is kept")
            .clone()
    }

    /// A second engine over the same Store, which knows nothing the first one holds.
    fn second_engine(&self) -> Reviews {
        engine(&self.scratch, self.store.clone())
    }

    fn base_moved(&self, from: &CommitId, to: &CommitId) -> Result<Vec<ThreadId>, EngineError> {
        self.reviews
            .base_moved(&workspace(), &self.worktree, from, to)
    }
}

fn place(tree: &CommitId, path: &str, first: u32, last: u32) -> ThreadPlace {
    ThreadPlace {
        tree: tree.as_str().to_owned(),
        path: PathBuf::from(path),
        side: Side::Old,
        first,
        last,
        notice: None,
    }
}

/// `line 1` to `line 6` with `inserted` put before line `at` (counted from 1).
fn with_before(at: usize, inserted: &[&str]) -> String {
    let mut lines: Vec<String> = (1..=6).map(|i| format!("line {i}")).collect();
    for (n, text) in inserted.iter().enumerate() {
        lines.insert(at - 1 + n, (*text).to_owned());
    }
    lines.iter().map(|l| format!("{l}\n")).collect()
}

// THREAD-09
#[test]
fn a_thread_on_a_removed_line_is_carried_to_the_new_base_and_the_others_stay() {
    let world = World::new();
    // The branch removes line 3, and the reviewer comments on it, on the old side.
    world.old_thread(1, &world.m1, "f", 3, "line 3");
    // A thread on the new side: it follows the states, not the base.
    write(
        &world.repo,
        "f",
        "line 1\nline 2\nline 4\nline 5\nline 6\nadded\n",
    );
    world
        .reviews
        .save_draft(
            Some(&world.worktree),
            &world.review,
            &new_thread(2, "f", 6, "added", "on the new side"),
        )
        .expect("save the draft");
    let new_side = world.thread(2);
    // A resolved thread on the old side, and one on the old side placed on a state.
    world.old_thread(3, &world.m1, "f", 3, "line 3");
    world
        .store
        .set_resolved(&ThreadId::new(id(3)), true)
        .expect("resolve");
    let resolved = world.thread(3);
    let state = world
        .reviews
        .states_of(&workspace())
        .expect("the state store")
        .record(&world.worktree)
        .expect("record");
    let on_a_state = Thread {
        id: ThreadId::new(id(4)),
        anchor: Anchor {
            tree: state.id.as_str().to_owned(),
            path: PathBuf::from("f"),
            side: Side::Old,
            first: 3,
            last: 3,
            text: "line 3".to_owned(),
        },
        resolved: false,
        addressed: false,
        comments: vec![Comment {
            id: CommentId::new(id(1004)),
            author: Author::Reviewer,
            text: "on a state".to_owned(),
            round: None,
        }],
        now: None,
    };
    world
        .store
        .put_thread(&world.review, &on_a_state)
        .expect("keep");
    let on_a_state = world.thread(4);
    git(&world.repo, &["checkout", "--quiet", "--", "f"]);
    // A thread on the new side that sits on the very commit the base moves from: only its
    // side keeps it out, since no recorded state tells it apart from a thread of the base.
    world
        .reviews
        .save_draft(
            Some(&world.worktree),
            &world.review,
            &DraftSave {
                thread: ThreadId::new(id(5)),
                comment: CommentId::new(id(1005)),
                on: DraftOn::NewThread(NewAnchor {
                    tree: Some(world.m1.as_str().to_owned()),
                    path: PathBuf::from("f"),
                    side: Side::New,
                    first: 3,
                    last: 3,
                    text: "line 3".to_owned(),
                }),
                text: "on the new side of the base".to_owned(),
            },
        )
        .expect("save the draft");
    let new_side_on_the_base = world.thread(5);
    assert_eq!(new_side_on_the_base.anchor.tree, world.m1.as_str());

    // A thread on the old side above the lines `main` gains: its lines stay.
    world.old_thread(6, &world.m1, "f", 1, "line 1");

    // `main` gains two lines above line 3.
    let m2 = world.move_main(&[("f", Some(&with_before(3, &["new 1", "new 2"])))]);
    let moved = world.base_moved(&world.m1, &m2).expect("carry");

    assert_eq!(moved.len(), 2, "{moved:?}");
    assert!(moved.contains(&ThreadId::new(id(1))), "{moved:?}");
    assert!(moved.contains(&ThreadId::new(id(6))), "{moved:?}");
    let carried = world.thread(1);
    assert_eq!(carried.now, Some(place(&m2, "f", 5, 5)));
    assert_eq!(world.thread(6).now, Some(place(&m2, "f", 1, 1)));
    assert_eq!(carried.anchor.tree, world.m1.as_str(), "the anchor is kept");
    assert_eq!(world.thread(2), new_side);
    assert_eq!(world.thread(5), new_side_on_the_base);
    assert_eq!(world.thread(3), resolved);
    assert_eq!(world.thread(4), on_a_state);
}

#[test]
fn a_thread_whose_lines_did_not_move_is_carried_too_so_that_the_next_move_finds_it() {
    let world = World::new();
    // Line 1 is above the two lines `main` gains, so its place does not change.
    world.old_thread(1, &world.m1, "f", 1, "line 1");
    let m2 = world.move_main(&[("f", Some(&with_before(3, &["new 1", "new 2"])))]);
    // A move after it puts a line above line 1.
    let m3 = world.move_main(&[(
        "f",
        Some(&format!("top\n{}", with_before(3, &["new 1", "new 2"]))),
    )]);

    assert_eq!(
        world.base_moved(&world.m1, &m2).expect("carry"),
        vec![ThreadId::new(id(1))]
    );
    assert_eq!(world.thread(1).now, Some(place(&m2, "f", 1, 1)));
    assert_eq!(
        world.base_moved(&m2, &m3).expect("carry"),
        vec![ThreadId::new(id(1))]
    );
    assert_eq!(world.thread(1).now, Some(place(&m3, "f", 2, 2)));
}

#[test]
fn a_thread_carried_once_is_carried_again_from_where_it_sits_and_not_twice_from_one_commit() {
    let world = World::new();
    world.old_thread(1, &world.m1, "f", 3, "line 3");
    let m2 = world.move_main(&[("f", Some(&with_before(1, &["above"])))]);
    let m3 = world.move_main(&[("f", Some(&format!("two\nabove\n{}", numbered(6))))]);

    assert_eq!(
        world.base_moved(&world.m1, &m2).expect("carry"),
        vec![ThreadId::new(id(1))]
    );
    assert_eq!(world.thread(1).now, Some(place(&m2, "f", 4, 4)));
    // The same move again finds nothing placed on `m1`.
    assert_eq!(world.base_moved(&world.m1, &m2).expect("carry"), vec![]);
    // The next move starts from where the thread sits now.
    assert_eq!(
        world.base_moved(&m2, &m3).expect("carry"),
        vec![ThreadId::new(id(1))]
    );
    assert_eq!(world.thread(1).now, Some(place(&m3, "f", 5, 5)));
}

#[test]
fn a_thread_on_a_line_the_base_rewrote_says_so_and_one_on_a_deleted_file_says_it_is_gone() {
    let world = World::new();
    write(&world.repo, "g", &numbered(4));
    git(&world.repo, &["add", "g"]);
    git(&world.repo, &["commit", "--quiet", "-m", "second file"]);
    let m1 = CommitId::new(git(&world.repo, &["rev-parse", "HEAD"]));
    world.old_thread(1, &m1, "f", 3, "line 3");
    world.old_thread(2, &m1, "g", 2, "line 2");
    git(&world.repo, &["checkout", "--quiet", "main"]);
    git(&world.repo, &["merge", "--quiet", "--ff-only", "b"]);
    git(&world.repo, &["checkout", "--quiet", "b"]);

    let m2 = world.move_main(&[
        (
            "f",
            Some("line 1\nline 2\nrewritten entirely\nline 4\nline 5\nline 6\n"),
        ),
        ("g", None),
    ]);
    world.base_moved(&m1, &m2).expect("carry");

    assert_eq!(
        world.thread(1).now,
        Some(ThreadPlace {
            notice: Some(NoticeRecord::Changed),
            ..place(&m2, "f", 3, 3)
        })
    );
    assert_eq!(
        world.thread(2).now,
        Some(ThreadPlace {
            notice: Some(NoticeRecord::Deleted),
            ..place(&m2, "g", 2, 2)
        })
    );
}

#[test]
fn a_thread_follows_its_file_to_a_new_name_through_the_base() {
    let world = World::new();
    world.old_thread(1, &world.m1, "f", 4, "line 4");
    git(&world.repo, &["checkout", "--quiet", "main"]);
    git(&world.repo, &["mv", "f", "renamed"]);
    let m2 = CommitId::new(commit_all(&world.repo, "rename"));
    git(&world.repo, &["checkout", "--quiet", "b"]);

    world.base_moved(&world.m1, &m2).expect("carry");

    assert_eq!(world.thread(1).now, Some(place(&m2, "renamed", 4, 4)));
}

#[test]
fn a_base_that_did_not_move_carries_nothing_and_a_commit_the_repository_lacks_is_an_error() {
    let world = World::new();
    world.old_thread(1, &world.m1, "f", 3, "line 3");

    assert_eq!(
        world.base_moved(&world.m1, &world.m1).expect("carry"),
        vec![]
    );
    assert_eq!(world.thread(1).now, None);

    let unknown = CommitId::new("2".repeat(40));
    assert!(world.base_moved(&world.m1, &unknown).is_err());
    assert_eq!(world.thread(1).now, None, "a failed carry moves nothing");
}

#[test]
fn a_thread_on_a_file_outside_the_worktree_is_not_touched() {
    let world = World::new();
    let save = DraftSave {
        thread: ThreadId::new(id(1)),
        comment: CommentId::new(id(1001)),
        on: DraftOn::NewThread(NewAnchor {
            tree: None,
            path: world.scratch.join("outside.txt"),
            side: Side::Old,
            first: 1,
            last: 1,
            text: "x".to_owned(),
        }),
        text: "outside".to_owned(),
    };
    world
        .reviews
        .save_draft(Some(&world.worktree), &world.review, &save)
        .expect("save the draft");
    // A thread of the old side, placed on the base itself, whose path is absolute: only
    // its path keeps it out, which a draft cannot give it (it is written on the new side,
    // with no tree).
    let outside = world.scratch.join("outside.txt");
    world
        .store
        .put_thread(
            &world.review,
            &Thread {
                id: ThreadId::new(id(2)),
                anchor: Anchor {
                    tree: world.m1.as_str().to_owned(),
                    path: outside,
                    side: Side::Old,
                    first: 1,
                    last: 1,
                    text: "x".to_owned(),
                },
                resolved: false,
                addressed: false,
                comments: vec![Comment {
                    id: CommentId::new(id(1002)),
                    author: Author::Reviewer,
                    text: "outside, on the base".to_owned(),
                    round: None,
                }],
                now: None,
            },
        )
        .expect("keep");
    let before = world.thread(1);
    let on_the_base = world.thread(2);
    let m2 = world.move_main(&[("f", Some(&with_before(1, &["above"])))]);

    assert_eq!(world.base_moved(&world.m1, &m2).expect("carry"), vec![]);
    assert_eq!(world.thread(1), before);
    assert_eq!(world.thread(2), on_the_base);
}

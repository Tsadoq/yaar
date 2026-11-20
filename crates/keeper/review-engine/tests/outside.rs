//! A thread on a file outside the worktree, or one inside it that git ignores, reached
//! through go to definition: kept with its path, lines and text and no version, with no
//! state recorded for it, and never carried through a diff. Through the engine with the real
//! Store and the real Recorder beneath, on repositories the test makes.

mod support;

use std::path::{Path, PathBuf};

use support::{Scratch, commit_all, engine, id, init, new_thread, open, store, workspace, write};
use yaar_recorder::{StateId, Worktree};
use yaar_review_engine::{
    CommentId, DraftOn, DraftSave, EngineError, NewAnchor, Place, Placed, ReviewError, ReviewId,
    Reviews, Saved, Side, Thread, ThreadId,
};
use yaar_store::Store;

/// What the tests share: a repository whose `a.txt` holds ten lines, a file `lib/x.rs` of
/// five lines in a folder beside it (outside every worktree), the review of the workspace
/// and the engine.
struct World {
    scratch: Scratch,
    store: Store,
    reviews: Reviews,
    repo: PathBuf,
    worktree: Worktree,
    review: ReviewId,
}

impl World {
    fn new() -> World {
        let scratch = Scratch::new();
        let repo = scratch.join("repo");
        init(&repo);
        write(&repo, "a.txt", &numbered("line", 10));
        commit_all(&repo, "first");
        write(&scratch.join("lib"), "x.rs", &numbered("fn", 5));
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

    /// The file of the folder beside the worktree.
    fn outside(&self) -> PathBuf {
        self.scratch.join("lib/x.rs")
    }

    /// How many states the state store of the workspace holds.
    fn states(&self) -> usize {
        self.reviews
            .states_of(&workspace())
            .expect("the state store")
            .states()
            .expect("the states")
            .len()
    }

    /// Records a state of the files as they are, without the engine, and follows it.
    fn record_and_follow(&self) -> (StateId, Vec<ThreadId>) {
        let state = self
            .reviews
            .states_of(&workspace())
            .expect("the state store")
            .record(&self.worktree)
            .expect("record")
            .id;
        let moved = self
            .reviews
            .follow(&workspace(), &state)
            .expect("following does not fail");
        (state, moved)
    }

    /// One thread of the review as the Store keeps it, read by a second engine.
    fn thread(&self, n: u128) -> Thread {
        engine(&self.scratch, self.store.clone())
            .review(&self.review)
            .expect("read the review")
            .thread(&ThreadId::new(id(n)))
            .expect("the thread is kept")
            .clone()
    }
}

/// The lines `<word> 1` to `<word> n`.
fn numbered(word: &str, n: u32) -> String {
    (1..=n).map(|i| format!("{word} {i}\n")).collect()
}

/// The save that starts thread `n` on lines `first` to `last` of `path` as `side`, with
/// `tree` and the code the lines hold.
fn save_on(
    n: u128,
    tree: Option<&str>,
    path: &Path,
    side: Side,
    (first, last): (u32, u32),
    code: &str,
) -> DraftSave {
    DraftSave {
        thread: ThreadId::new(id(n)),
        comment: CommentId::new(id(n + 1000)),
        on: DraftOn::NewThread(NewAnchor {
            tree: tree.map(str::to_owned),
            path: path.to_owned(),
            side,
            first,
            last,
            text: code.to_owned(),
        }),
        text: format!("comment of {n}"),
    }
}

/// The save of thread `n` on line 3 of the file outside the worktree.
fn outside_save(world: &World, n: u128) -> DraftSave {
    save_on(n, None, &world.outside(), Side::New, (3, 3), "fn 3")
}

#[test]
fn a_new_thread_on_a_file_outside_the_worktree_is_kept_with_no_state_and_no_version() {
    let world = World::new();
    let before = world.states();

    let saved = world
        .reviews
        .save_draft(
            Some(&world.worktree),
            &world.review,
            &outside_save(&world, 1),
        )
        .expect("save the draft");

    assert_eq!(saved, Saved::Made);
    assert_eq!(world.states(), before, "no state was recorded for it");
    let thread = world.thread(1);
    assert_eq!(thread.anchor.path, world.outside());
    assert_eq!((thread.anchor.first, thread.anchor.last), (3, 3));
    assert_eq!(thread.anchor.text, "fn 3");
    assert_eq!(thread.anchor.side, Side::New);
    assert_eq!(thread.anchor.tree, "", "the version is empty");
    assert_eq!(thread.comments[0].text, "comment of 1");
    assert_eq!(thread.now, None);
}

#[test]
fn a_thread_on_a_file_outside_the_worktree_is_kept_on_the_new_side_whatever_the_save_gave() {
    // A file outside the worktree has one side: there is no old version of it to be on.
    let world = World::new();
    write(&world.repo, "ignored.log", "one\ntwo\n");
    for (n, path, line, code) in [
        (1, world.outside(), 3, "fn 3"),
        // The same for a file of the worktree that the Window names by its absolute path.
        (2, world.repo.join("ignored.log"), 2, "two"),
    ] {
        world
            .reviews
            .save_draft(
                None,
                &world.review,
                &save_on(n, None, &path, Side::Old, (line, line), code),
            )
            .expect("save the draft");

        assert_eq!(
            world.thread(n).anchor.side,
            Side::New,
            "the save of {} said Old",
            path.display()
        );
    }
    // A relative path keeps the side it was given: the rule is about the outside only.
    world
        .reviews
        .save_draft(
            Some(&world.worktree),
            &world.review,
            &save_on(3, None, Path::new("a.txt"), Side::Old, (1, 1), "line 1"),
        )
        .expect("save on the old side of a file of the worktree");
    assert_eq!(world.thread(3).anchor.side, Side::Old);
}

#[test]
fn a_new_thread_on_a_file_outside_the_worktree_needs_no_worktree() {
    let world = World::new();

    let saved = world
        .reviews
        .save_draft(None, &world.review, &outside_save(&world, 1))
        .expect("kept with no worktree");

    assert_eq!(saved, Saved::Made);
    assert_eq!(world.thread(1).anchor.path, world.outside());
    assert_eq!(world.states(), 0);
    // The rule of slice 3 stands for a relative path: lines of the files on disk need the
    // worktree.
    let relative = new_thread(2, "a.txt", 1, "line 1", "x");
    assert!(matches!(
        world.reviews.save_draft(None, &world.review, &relative),
        Err(EngineError::NoWorktree)
    ));
}

#[test]
fn a_thread_on_a_file_outside_the_worktree_stays_where_it_was_when_states_are_recorded() {
    let world = World::new();
    world
        .reviews
        .save_draft(
            Some(&world.worktree),
            &world.review,
            &outside_save(&world, 1),
        )
        .expect("save the outside thread");
    // A thread inside the worktree, which does move, shows that the following ran.
    world
        .reviews
        .save_draft(
            Some(&world.worktree),
            &world.review,
            &new_thread(2, "a.txt", 8, "line 8", "inside"),
        )
        .expect("save the inside thread");
    write(
        &world.repo,
        "a.txt",
        &format!("added\n{}", numbered("line", 10)),
    );
    // What the file outside holds now, or whether it exists, is of no concern to the thread.
    std::fs::remove_file(world.outside()).expect("remove the file outside");

    let (state, moved) = world.record_and_follow();

    assert_eq!(
        moved,
        vec![ThreadId::new(id(2))],
        "only the inside thread moved"
    );
    assert_eq!(
        world.thread(2).now.map(|now| (now.tree, now.first)),
        Some((state.as_str().to_owned(), 9))
    );
    let outside = world.thread(1);
    assert_eq!(outside.now, None, "no place, no notice");
    assert_eq!(outside.anchor.path, world.outside());
    assert_eq!((outside.anchor.first, outside.anchor.last), (3, 3));
    assert_eq!(outside.anchor.tree, "");
}

#[test]
fn a_state_the_engine_takes_does_not_fail_the_following_of_a_thread_outside_the_worktree() {
    let world = World::new();
    world
        .reviews
        .save_draft(None, &world.review, &outside_save(&world, 1))
        .expect("save");
    write(&world.repo, "a.txt", &numbered("line", 4));

    let state = world
        .reviews
        .mark_reviewed(&world.worktree, &world.review)
        .expect("mark reviewed");

    // Asked directly, the following gives no failure, as it would for a thread whose tree
    // the store cannot read.
    assert_eq!(
        world.reviews.follow(&workspace(), &state).expect("follow"),
        vec![]
    );
    assert_eq!(world.thread(1).now, None);
}

#[test]
fn a_place_outside_the_worktree_is_left_where_it_is_when_carried_between_two_trees() {
    // Nothing is read for it: the two trees are ones the state store never held.
    let world = World::new();
    let placed = Placed {
        at: Place {
            path: world.outside(),
            side: Side::New,
            first: 3,
            last: 3,
        },
        notice: None,
    };
    let unheld = "1".repeat(40);

    let carried = world
        .reviews
        .carry_between(&workspace(), &placed, &unheld, &"2".repeat(40))
        .expect("nothing is read for it");

    assert_eq!(carried, placed);
}

#[test]
fn a_path_that_is_absolute_and_inside_the_worktree_is_kept_as_given() {
    let world = World::new();
    write(&world.repo, "ignored.log", "one\ntwo\n");
    let inside = world.repo.join("ignored.log");
    let before = world.states();

    world
        .reviews
        .save_draft(
            Some(&world.worktree),
            &world.review,
            &save_on(1, None, &inside, Side::New, (2, 2), "two"),
        )
        .expect("save");

    assert_eq!(world.states(), before);
    let thread = world.thread(1);
    assert_eq!(thread.anchor.path, inside, "not made relative");
    assert_eq!(thread.anchor.tree, "");
}

#[test]
fn an_absolute_path_decides_even_when_the_save_names_a_commit() {
    let world = World::new();
    // A commit the state store never held: keeping it from the worktree would fail.
    let unheld = "1".repeat(40);

    world
        .reviews
        .save_draft(
            Some(&world.worktree),
            &world.review,
            &save_on(
                1,
                Some(&unheld),
                &world.outside(),
                Side::New,
                (3, 3),
                "fn 3",
            ),
        )
        .expect("the path decides: no commit is kept");

    assert_eq!(world.thread(1).anchor.tree, "");
    assert_eq!(world.states(), 0);
}

#[test]
fn the_lines_of_a_thread_outside_the_worktree_are_checked_as_those_of_any_anchor() {
    let world = World::new();
    let outside = world.outside();
    let wrong = [
        // The first line is 0.
        save_on(1, None, &outside, Side::New, (0, 1), "fn 1"),
        // The last line is before the first.
        save_on(2, None, &outside, Side::New, (4, 3), "fn 4"),
        // Two lines, one line of text.
        save_on(3, None, &outside, Side::New, (3, 4), "fn 3"),
        // One line, two lines of text.
        save_on(4, None, &outside, Side::New, (3, 3), "fn 3\nfn 4"),
    ];
    for save in &wrong {
        assert!(
            matches!(
                world.reviews.save_draft(None, &world.review, save),
                Err(EngineError::Refused(ReviewError::BadAnchor))
            ),
            "{:?} is refused",
            save.on
        );
    }
    assert!(
        world
            .reviews
            .review(&world.review)
            .expect("read")
            .threads()
            .is_empty(),
        "nothing was kept"
    );
    // A range of several lines is kept.
    world
        .reviews
        .save_draft(
            None,
            &world.review,
            &save_on(5, None, &outside, Side::New, (3, 4), "fn 3\nfn 4"),
        )
        .expect("two lines");
    assert_eq!(world.thread(5).anchor.last, 4);
}

#[test]
fn a_thread_outside_the_worktree_takes_replies_and_edits_as_any_thread() {
    let world = World::new();
    let save = outside_save(&world, 1);
    world
        .reviews
        .save_draft(None, &world.review, &save)
        .expect("start");

    // The same comment again: its text changes, which is the save of each keystroke.
    let edited = DraftSave {
        text: "edited".to_owned(),
        ..save.clone()
    };
    assert_eq!(
        world
            .reviews
            .save_draft(None, &world.review, &edited)
            .expect("edit"),
        Saved::Text
    );
    assert_eq!(world.thread(1).comments[0].text, "edited");
    assert_eq!(world.thread(1).anchor.tree, "");

    // Once the comment is sent, the thread takes a reply.
    world
        .reviews
        .copy_as_text(&world.worktree, &world.review)
        .expect("send the comment");
    let reply = DraftSave {
        thread: save.thread.clone(),
        comment: CommentId::new(id(2000)),
        on: DraftOn::Thread,
        text: "a reply".to_owned(),
    };
    assert_eq!(
        world
            .reviews
            .save_draft(None, &world.review, &reply)
            .expect("reply"),
        Saved::Made
    );
    let kept = world.thread(1);
    assert_eq!(kept.comments.len(), 2);
    assert_eq!(kept.comments[1].text, "a reply");
    assert_eq!(kept.anchor.tree, "");
}

#[test]
fn a_thread_outside_the_worktree_is_still_there_after_a_restart() {
    let world = World::new();
    world
        .reviews
        .save_draft(None, &world.review, &outside_save(&world, 1))
        .expect("save");

    let again = engine(&world.scratch, store(&world.scratch));
    let view = again.of_workspace(&workspace(), None).expect("read");

    let thread = view.reviews[0].thread(&ThreadId::new(id(1))).expect("kept");
    assert_eq!(thread.anchor.path, world.outside());
    assert_eq!((thread.anchor.first, thread.anchor.last), (3, 3));
    assert_eq!(thread.anchor.text, "fn 3");
    assert_eq!(thread.anchor.tree, "");
    assert_eq!(thread.now, None);
}

//! A round shown as sent until its agent's first edit, and what changed since a round's
//! comments were sent, through the engine with the real Store and Recorder on a repository
//! the test makes.

mod support;

use std::path::{Path, PathBuf};

use support::{
    Scratch, commit_all, engine, git_bytes, id, init, new_thread, open, session, store, workspace,
    write,
};
use yaar_recorder::{Change, ChangedPath};
use yaar_review_engine::{EngineError, ReviewError, ReviewId, Reviews, SentBy};
use yaar_store::{CommentId, Round};

// What git says changed between two states, from the state store's own
// `git diff-tree -r -M --name-status`.
fn git_says(reviews: &Reviews, from: &str, to: &str) -> Vec<ChangedPath> {
    let states = reviews.states_of(&workspace()).expect("the state store");
    let printed = git_bytes(
        Path::new("/"),
        &[
            "--git-dir",
            states.folder().to_str().expect("a path of text"),
            "diff-tree",
            "-r",
            "-M",
            "--name-status",
            from,
            to,
        ],
    );
    String::from_utf8(printed)
        .expect("git printed text")
        .lines()
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            match fields[0].chars().next().expect("a status letter") {
                'A' => ChangedPath {
                    path: PathBuf::from(fields[1]),
                    change: Change::Added,
                },
                'M' => ChangedPath {
                    path: PathBuf::from(fields[1]),
                    change: Change::Modified,
                },
                'D' => ChangedPath {
                    path: PathBuf::from(fields[1]),
                    change: Change::Deleted,
                },
                'R' => ChangedPath {
                    path: PathBuf::from(fields[2]),
                    change: Change::Renamed {
                        from: PathBuf::from(fields[1]),
                    },
                },
                other => panic!("a status git gave that the test does not know: {other}"),
            }
        })
        .collect()
}

fn paths(changed: &[ChangedPath]) -> Vec<&str> {
    changed
        .iter()
        .map(|one| one.path.to_str().expect("a path of text"))
        .collect()
}

fn newest(reviews: &Reviews) -> String {
    reviews
        .states_of(&workspace())
        .expect("the state store")
        .states()
        .expect("list the states")
        .pop()
        .expect("a state")
        .id
        .as_str()
        .to_owned()
}

struct Setup {
    repo: PathBuf,
    reviews: Reviews,
    review: ReviewId,
    store: yaar_store::Store,
}

fn setup(scratch: &Scratch) -> Setup {
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "one\n");
    write(&repo, "b.txt", "two\n");
    write(&repo, "c.txt", "three\n");
    write(
        &repo,
        "old.txt",
        "a file with enough lines\nthat git pairs it with its new name\nwhen it is moved\nand nothing else changes\n",
    );
    write(&repo, "gone.txt", "a file that goes\n");
    commit_all(&repo, "first");
    let store = store(scratch);
    let reviews = engine(scratch, store.clone());
    let review = reviews
        .of_workspace(&workspace(), Some(&open(&repo)))
        .expect("read")
        .reviews[0]
        .id()
        .clone();
    Setup {
        repo,
        reviews,
        review,
        store,
    }
}

fn send_round(on: &Setup, thread: u128, path: &str, line_text: &str) -> String {
    let worktree = open(&on.repo);
    on.reviews
        .save_draft(
            Some(&worktree),
            &on.review,
            &new_thread(thread, path, 1, line_text, "a comment"),
        )
        .expect("save the draft");
    on.reviews
        .copy_as_text(&worktree, &on.review)
        .expect("send the round");
    let round = on
        .reviews
        .review(&on.review)
        .expect("read")
        .rounds()
        .last()
        .cloned()
        .expect("a round");
    round.state.expect("a copy records the state of its round")
}

#[test]
fn what_changed_since_a_round_is_the_diff_from_the_state_recorded_at_its_send() {
    let scratch = Scratch::new();
    let on = setup(&scratch);
    let worktree = open(&on.repo);

    let first = send_round(&on, 1, "a.txt", "one");
    write(&on.repo, "a.txt", "one, changed\n");
    write(&on.repo, "b.txt", "two, changed\n");
    let second = send_round(&on, 2, "b.txt", "two, changed");
    assert_ne!(first, second);
    write(&on.repo, "c.txt", "three, changed\n");
    write(&on.repo, "a.txt", "one, changed twice\n");
    let old = std::fs::read_to_string(on.repo.join("old.txt")).expect("read");
    std::fs::remove_file(on.repo.join("old.txt")).expect("remove");
    write(&on.repo, "moved.txt", &old);
    std::fs::remove_file(on.repo.join("gone.txt")).expect("remove");

    let states_before = on
        .reviews
        .states_of(&workspace())
        .expect("the state store")
        .states()
        .expect("list the states")
        .len();
    let since_first = on
        .reviews
        .changed_since_round(&worktree, &on.review, 1)
        .expect("what changed since round 1");
    // The worktree was recorded by the call, as a state of its own: the one the diff ends at.
    let now = newest(&on.reviews);
    assert_eq!(
        on.reviews
            .states_of(&workspace())
            .expect("the state store")
            .states()
            .expect("list the states")
            .len(),
        states_before + 1
    );
    assert_ne!(now, second);

    assert_eq!(
        paths(&since_first),
        ["a.txt", "b.txt", "c.txt", "gone.txt", "moved.txt"]
    );
    assert!(
        since_first.contains(&ChangedPath {
            path: PathBuf::from("moved.txt"),
            change: Change::Renamed {
                from: PathBuf::from("old.txt")
            },
        }),
        "{since_first:?}"
    );
    assert!(since_first.contains(&ChangedPath {
        path: PathBuf::from("gone.txt"),
        change: Change::Deleted
    }));
    assert_eq!(since_first, git_says(&on.reviews, &first, &now));

    let since_second = on
        .reviews
        .changed_since_round(&worktree, &on.review, 2)
        .expect("what changed since round 2");
    assert_eq!(
        paths(&since_second),
        ["a.txt", "c.txt", "gone.txt", "moved.txt"]
    );
    assert_eq!(since_second, git_says(&on.reviews, &second, &now));
    assert_eq!(newest(&on.reviews), now);
}

#[test]
fn nothing_changed_since_a_round_when_the_files_are_as_they_were_at_its_send() {
    let scratch = Scratch::new();
    let on = setup(&scratch);
    // The files differ from the commit and from every state before the send, and not from the
    // state of the send: only that counts.
    write(&on.repo, "a.txt", "one, changed before the send\n");
    send_round(&on, 1, "a.txt", "one, changed before the send");

    let changed = on
        .reviews
        .changed_since_round(&open(&on.repo), &on.review, 1)
        .expect("what changed since round 1");

    assert_eq!(changed, Vec::new());
}

#[test]
fn a_commit_made_after_the_send_changes_nothing_of_what_changed_since_it() {
    let scratch = Scratch::new();
    let on = setup(&scratch);
    let worktree = open(&on.repo);
    send_round(&on, 1, "a.txt", "one");
    write(&on.repo, "a.txt", "one, changed\n");
    write(&on.repo, "new.txt", "a new file\n");
    let uncommitted = on
        .reviews
        .changed_since_round(&worktree, &on.review, 1)
        .expect("what changed");

    commit_all(&on.repo, "the agent's work");
    let committed = on
        .reviews
        .changed_since_round(&worktree, &on.review, 1)
        .expect("what changed after the commit");

    assert_eq!(paths(&uncommitted), ["a.txt", "new.txt"]);
    assert_eq!(committed, uncommitted);
}

#[test]
fn a_round_with_no_state_or_none_at_all_is_unknown_and_nothing_is_recorded_for_it() {
    let scratch = Scratch::new();
    let on = setup(&scratch);
    let worktree = open(&on.repo);
    send_round(&on, 1, "a.txt", "one");
    // A round delivered with a message has no state until it is recorded: made here as the
    // Store keeps one.
    on.reviews
        .save_draft(
            Some(&worktree),
            &on.review,
            &new_thread(2, "b.txt", 1, "two", "another comment"),
        )
        .expect("save the draft");
    on.store
        .put_round(
            &on.review,
            &Round {
                number: 2,
                state: None,
                sent_by: SentBy::Delivered,
                edited: false,
            },
            &[CommentId::new(id(1002))],
        )
        .expect("keep a round with no state");
    write(&on.repo, "a.txt", "one, changed\n");
    let known = on
        .reviews
        .states_of(&workspace())
        .expect("the state store")
        .states()
        .expect("list the states")
        .len();

    for round in [2, 3, 0] {
        let refused = on.reviews.changed_since_round(&worktree, &on.review, round);
        assert!(
            matches!(
                refused,
                Err(EngineError::Refused(ReviewError::UnknownRound(n))) if n == round
            ),
            "round {round}: {refused:?}"
        );
    }
    let no_review = on
        .reviews
        .changed_since_round(&worktree, &ReviewId::new(id(0x999)), 1);
    assert!(
        matches!(no_review, Err(EngineError::UnknownReview(_))),
        "{no_review:?}"
    );
    assert_eq!(
        on.reviews
            .states_of(&workspace())
            .expect("the state store")
            .states()
            .expect("list the states")
            .len(),
        known
    );
}

#[test]
fn an_edit_of_a_session_is_kept_on_the_newest_round_of_its_review_and_on_no_other() {
    let scratch = Scratch::new();
    let on = setup(&scratch);
    let first = session(&on.store, 1);
    let second = session(&on.store, 2);
    let review_one = on.reviews.session_started(&first).expect("start");
    let review_two = on.reviews.session_started(&second).expect("start");
    assert_ne!(review_one, review_two);

    assert_eq!(on.reviews.agent_edited(&first.id).expect("edited"), None);
    let nobody = yaar_review_engine::SessionId::new(id(0x777));
    assert_eq!(on.reviews.agent_edited(&nobody).expect("edited"), None);

    let worktree = open(&on.repo);
    on.reviews
        .save_draft(
            Some(&worktree),
            &review_one,
            &new_thread(1, "a.txt", 1, "one", "a comment"),
        )
        .expect("save the draft");
    on.reviews
        .copy_as_text(&worktree, &review_one)
        .expect("send");
    let round_of = |reviews: &Reviews, review: &ReviewId| {
        let read = reviews.review(review).expect("read");
        (read.shows_sent(1), read.rounds().first().map(|r| r.edited))
    };
    assert_eq!(round_of(&on.reviews, &review_one), (true, Some(false)));

    assert_eq!(on.reviews.agent_edited(&second.id).expect("edited"), None);
    assert_eq!(round_of(&on.reviews, &review_one), (true, Some(false)));

    assert_eq!(
        on.reviews.agent_edited(&first.id).expect("edited"),
        Some((review_one.clone(), 1))
    );
    assert_eq!(on.reviews.agent_edited(&first.id).expect("edited"), None);

    drop(on.reviews);
    drop(on.store);
    let again = engine(&scratch, self::store(&scratch));
    assert_eq!(round_of(&again, &review_one), (false, Some(true)));
}

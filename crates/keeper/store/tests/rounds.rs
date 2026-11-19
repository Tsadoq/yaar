//! Component tests: the calls of the Store that keep a round, its state and its edit
//! mark, and the attachment a send takes away, on a real database file.

mod support;

use std::path::PathBuf;

use support::Scratch;
use yaar_store::{
    Anchor, AttachedRecord, Author, Comment, CommentId, ReviewId, ReviewRecord, Round, SentBy,
    Side, Store, StoreError, Thread, ThreadId, WorkspaceId,
};

/// An id of 32 hexadecimal digits, from a number.
fn id(n: u128) -> String {
    format!("{n:032x}")
}

fn cid(n: u128) -> CommentId {
    CommentId::new(id(n))
}

fn tid(n: u128) -> ThreadId {
    ThreadId::new(id(n))
}

fn rid(n: u128) -> ReviewId {
    ReviewId::new(id(n))
}

fn thread(n: u128, first_comment: u128, text: &str) -> Thread {
    Thread {
        id: tid(n),
        anchor: Anchor {
            tree: "a".repeat(40),
            path: PathBuf::from("src/lib.rs"),
            side: Side::New,
            first: 3,
            last: 3,
            text: "let x = 1;".to_owned(),
        },
        resolved: false,
        addressed: false,
        comments: vec![Comment {
            id: cid(first_comment),
            author: Author::Reviewer,
            text: text.to_owned(),
            round: None,
        }],
        now: None,
    }
}

/// A round delivered with a message, with no state yet.
fn round(number: u32) -> Round {
    Round {
        number,
        state: None,
        sent_by: SentBy::Delivered,
        edited: false,
    }
}

/// A review `1` with three threads, `10`, `11` and `12`, each holding one draft (`100`,
/// `110`, `120`), and a first round that sent the draft of `10`, answered by the agent
/// with the addressed mark; `11` is addressed too, though nothing of it was sent.
fn review_with_marks(store: &Store) {
    store
        .put_review(&ReviewRecord {
            id: rid(1),
            workspace: WorkspaceId::new(id(1)),
            agent: Some("session".to_owned()),
            reviewed: None,
            threads: vec![],
            rounds: vec![],
            attached: None,
        })
        .expect("put review");
    for (n, first) in [(10, 100), (11, 110), (12, 120)] {
        store
            .put_thread(&rid(1), &thread(n, first, "text"))
            .expect("put thread");
    }
    store
        .put_round(&rid(1), &round(1), &[cid(100), cid(110)])
        .expect("put round 1");
    for n in [10, 11] {
        store
            .put_reply(
                &tid(n),
                &Comment {
                    id: cid(n * 1000),
                    author: Author::Agent,
                    text: "done".to_owned(),
                    round: Some(1),
                },
                true,
            )
            .expect("put reply");
    }
}

fn review_of(store: &Store) -> ReviewRecord {
    store
        .review(&rid(1))
        .expect("read the review")
        .expect("the review is kept")
}

fn addressed(store: &Store) -> Vec<bool> {
    review_of(store)
        .threads
        .iter()
        .map(|t| t.addressed)
        .collect()
}

fn refusal_of<T: std::fmt::Debug>(result: Result<T, StoreError>) -> String {
    match result {
        Err(StoreError::Database(sentence)) => sentence,
        other => panic!("not refused with a sentence: {other:?}"),
    }
}

/// How many bytes the write-ahead file holds; none when it is not there.
fn wal_len(scratch: &Scratch) -> u64 {
    std::fs::metadata(scratch.0.join("store.db-wal")).map_or(0, |m| m.len())
}

#[test]
fn a_round_takes_the_addressed_mark_of_the_threads_it_sends_and_of_no_other_and_leaves_the_attachment()
 {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    review_with_marks(&store);
    assert_eq!(addressed(&store), [true, true, false]);
    // A thread 12 the agent marked too, with a comment of the reviewer's that was not
    // sent: send a follow-up on 10 and 12, and 11 keeps its mark.
    store
        .put_comment(
            &tid(10),
            &Comment {
                id: cid(101),
                author: Author::Reviewer,
                text: "follow-up".to_owned(),
                round: None,
            },
        )
        .expect("put comment");
    store
        .put_reply(
            &tid(12),
            &Comment {
                id: cid(1200),
                author: Author::Agent,
                text: "unasked".to_owned(),
                round: Some(1),
            },
            true,
        )
        .expect("put reply");
    assert_eq!(addressed(&store), [true, true, true]);
    store
        .set_attached(&rid(1), Some(&AttachedRecord::Review))
        .expect("attach");

    // A round that is refused takes no mark.
    refusal_of(store.put_round(&rid(1), &round(3), &[cid(101)]));
    refusal_of(store.put_round(&rid(1), &round(2), &[cid(101), cid(10_000)]));
    assert_eq!(addressed(&store), [true, true, true]);

    store
        .put_round(&rid(1), &round(2), &[cid(101), cid(120)])
        .expect("put round 2");
    assert_eq!(addressed(&store), [false, true, false]);
    // The agent's replies of the thread stay, and so does the attachment.
    let kept = review_of(&store);
    assert_eq!(kept.threads[0].comments.len(), 3);
    assert_eq!(kept.attached, Some(AttachedRecord::Review));
    assert_eq!(kept.rounds.last().map(|r| r.state.clone()), Some(None));
}

#[test]
fn a_send_that_detaches_keeps_the_round_and_takes_the_attachment_away_in_one_transaction() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    review_with_marks(&store);
    store
        .put_thread(&rid(1), &thread(13, 130, "late"))
        .expect("put thread");
    store
        .put_comment(
            &tid(10),
            &Comment {
                id: cid(101),
                author: Author::Reviewer,
                text: "follow-up".to_owned(),
                round: None,
            },
        )
        .expect("put comment");
    store
        .set_attached(&rid(1), Some(&AttachedRecord::Thread(tid(13))))
        .expect("attach");
    let before = review_of(&store);

    // A comment that is no draft, after a good one: neither the round nor the
    // detachment is kept, nor the mark taken.
    for bad in [cid(100), cid(10_000), cid(999)] {
        refusal_of(store.put_round_detached(&rid(1), &round(2), &[cid(101), bad]));
        assert_eq!(review_of(&store), before);
    }
    // A round that is not the next, and a review that is not kept.
    refusal_of(store.put_round_detached(&rid(1), &round(3), &[cid(101)]));
    refusal_of(store.put_round_detached(&rid(9), &round(1), &[cid(101)]));
    assert_eq!(review_of(&store), before);

    store
        .put_round_detached(&rid(1), &round(2), &[cid(101), cid(130)])
        .expect("send and detach");
    let kept = review_of(&store);
    assert_eq!(kept.attached, None);
    assert_eq!(
        kept.rounds,
        vec![
            Round {
                state: None,
                ..round(1)
            },
            round(2)
        ]
    );
    assert_eq!(addressed(&store), [false, true, false, false]);
    assert_eq!(kept.threads[0].comments[2].round, Some(2));
    assert_eq!(kept.threads[3].comments[0].round, Some(2));

    // The whole of it is in the database file alone before the call returned: a copy of
    // that file without its write-ahead file shows the round and no attachment, and a
    // Store opened anew on the original reads the same.
    assert_eq!(wal_len(&scratch), 0);
    let elsewhere = Scratch::new();
    std::fs::copy(scratch.file(), elsewhere.file()).expect("copy the file");
    let copy = Store::open(&elsewhere.file()).expect("open the copy");
    assert_eq!(review_of(&copy), kept);
    drop(store);
    let again = Store::open(&scratch.file()).expect("open again");
    assert_eq!(review_of(&again), kept);
}

#[test]
fn a_send_that_detaches_a_review_with_nothing_attached_is_a_plain_send() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    review_with_marks(&store);
    assert_eq!(review_of(&store).attached, None);
    store
        .put_comment(
            &tid(12),
            &Comment {
                id: cid(121),
                author: Author::Reviewer,
                text: "again".to_owned(),
                round: None,
            },
        )
        .expect("put comment");
    store
        .put_round_detached(&rid(1), &round(2), &[cid(120)])
        .expect("send with nothing attached");
    assert_eq!(review_of(&store).attached, None);
    assert_eq!(review_of(&store).rounds.len(), 2);
}

#[test]
fn the_state_of_one_round_is_set_while_another_round_still_has_none() {
    let scratch = Scratch::new();
    let with_state = Round {
        state: Some("b".repeat(40)),
        ..round(2)
    };
    {
        let store = Store::open(&scratch.file()).expect("open");
        review_with_marks(&store);
        // Round 1 never got its state, as when the Keeper stopped before it was recorded.
        store
            .put_round(&rid(1), &round(2), &[])
            .expect("put round 2");
        store
            .set_round_state(&rid(1), 2, &"b".repeat(40))
            .expect("set the state of round 2");
        assert_eq!(review_of(&store).rounds, vec![round(1), with_state.clone()]);
    }
    let again = Store::open(&scratch.file()).expect("open again");
    assert_eq!(review_of(&again).rounds, vec![round(1), with_state]);
}

#[test]
fn the_state_of_a_round_is_set_once_and_the_edit_mark_only_for_a_round_that_is_kept() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    review_with_marks(&store);
    assert_eq!(review_of(&store).rounds, vec![round(1)]);

    store
        .set_round_state(&rid(1), 1, &"b".repeat(40))
        .expect("set the state");
    let sentence = refusal_of(store.set_round_state(&rid(1), 1, &"c".repeat(40)));
    assert!(sentence.contains("state already"), "{sentence}");
    refusal_of(store.set_round_state(&rid(1), 2, &"c".repeat(40)));
    refusal_of(store.set_round_state(&rid(9), 1, &"c".repeat(40)));
    // A round kept with a state has it already.
    store
        .put_round(
            &rid(1),
            &Round {
                state: Some("d".repeat(40)),
                ..round(2)
            },
            &[],
        )
        .expect("put round 2");
    refusal_of(store.set_round_state(&rid(1), 2, &"c".repeat(40)));

    store
        .set_round_edited(&rid(1), 2)
        .expect("set the edit mark");
    // Setting it again is no fault: the agent edits more than once.
    store.set_round_edited(&rid(1), 2).expect("set it again");
    refusal_of(store.set_round_edited(&rid(1), 3));
    refusal_of(store.set_round_edited(&rid(9), 1));

    drop(store);
    let again = Store::open(&scratch.file()).expect("open again");
    assert_eq!(
        review_of(&again).rounds,
        vec![
            Round {
                state: Some("b".repeat(40)),
                ..round(1)
            },
            Round {
                state: Some("d".repeat(40)),
                edited: true,
                ..round(2)
            },
        ]
    );
}

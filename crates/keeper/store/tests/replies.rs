//! Component tests: the replies of the agent, kept through the Store's calls while the
//! reviewer types, on a real database file.

mod support;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use support::Scratch;
use yaar_store::{
    Anchor, Author, Comment, CommentId, ReviewId, ReviewRecord, Round, SentBy, Side, Store,
    StoreError, Thread, ThreadId, WorkspaceId,
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

/// A thread `n` with one draft of the reviewer, `first_comment`.
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

fn round(number: u32) -> Round {
    Round {
        number,
        state: Some(format!("{number:040x}")),
        sent_by: SentBy::Delivered,
        edited: false,
    }
}

/// A reply of the agent, comment `n`, in round `round`.
fn reply(n: u128, text: &str, round: u32) -> Comment {
    Comment {
        id: cid(n),
        author: Author::Agent,
        text: text.to_owned(),
        round: Some(round),
    }
}

/// A review `1` of workspace `1` with a thread `10` whose comment was sent in round 1, and
/// a thread `11` holding a draft.
fn sent_and_draft(store: &Store) {
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
    store
        .put_thread(&rid(1), &thread(10, 100, "sent"))
        .expect("put thread");
    store
        .put_thread(&rid(1), &thread(11, 110, "draft"))
        .expect("put thread");
    store
        .put_round(&rid(1), &round(1), &[cid(100)])
        .expect("put round");
}

fn review_of(store: &Store) -> ReviewRecord {
    store
        .review(&rid(1))
        .expect("read the review")
        .expect("the review is kept")
}

const REPLIES: u128 = 400;

/// How long a test waits for a fact that only becomes true.
const DEADLINE: Duration = Duration::from_secs(10);

#[test]
fn four_hundred_replies_arrive_through_the_calls_while_a_draft_is_typed_and_all_are_there() {
    let scratch = Scratch::new();
    let replies: Vec<Comment> = (0..REPLIES)
        .map(|k| {
            // Quotes, a line break and a multi-byte letter, so that a reply is not
            // mistaken for another by its length alone.
            reply(1000 + k, &format!("reply {k}: \"é\"\nsecond line"), 1)
        })
        .collect();
    let last_draft;
    {
        let store = Store::open(&scratch.file()).expect("open");
        sent_and_draft(&store);

        let (typing, typed_once) = mpsc::channel::<()>();
        let (replying, replied) = mpsc::channel::<()>();
        // How many saves have returned, for the replier to see that the typing went on.
        let saves = Arc::new(AtomicU32::new(0));
        let typer = {
            let store = store.clone();
            let saves = Arc::clone(&saves);
            std::thread::spawn(move || {
                let mut text = String::new();
                for i in 0_u32.. {
                    // One more letter each time, as a draft that is being typed.
                    text.push('x');
                    store.save_draft(&cid(110), &text).expect("save the draft");
                    saves.fetch_add(1, Ordering::SeqCst);
                    if i == 0 {
                        typing.send(()).expect("say the first save returned");
                    }
                    // The replies are done once this arrives, or once the replier is gone
                    // without saying so; the loop never waits.
                    if !matches!(replied.try_recv(), Err(mpsc::TryRecvError::Empty)) {
                        break;
                    }
                }
                text
            })
        };
        let replier = {
            let store = store.clone();
            let replies = replies.clone();
            std::thread::spawn(move || {
                // Only once the typing is under way, so that the two overlap.
                typed_once.recv().expect("the typer saved once");
                let mut saved_before = 0;
                for (k, comment) in replies.iter().enumerate() {
                    if k == replies.len() - 1 {
                        // Before the last reply, a save has returned since the first
                        // reply went in: the two overlapped. Looked at again until true,
                        // within a deadline.
                        let deadline = Instant::now() + DEADLINE;
                        while saves.load(Ordering::SeqCst) <= saved_before {
                            assert!(Instant::now() < deadline, "the typer saved no more");
                            std::thread::yield_now();
                        }
                    }
                    // Even replies mark the thread addressed, so the last one (odd) clears
                    // the mark: a call that only ever set it would leave it.
                    store
                        .put_reply(&tid(10), comment, k % 2 == 0)
                        .expect("put the reply");
                    if k == 0 {
                        // Read once the first reply is in, so that a save counted later
                        // returned between the first reply and the last.
                        saved_before = saves.load(Ordering::SeqCst);
                    }
                }
                replying.send(()).expect("say the replies are done");
            })
        };
        replier.join().expect("the replier ran");
        last_draft = typer.join().expect("the typer ran");
    }

    let store = Store::open(&scratch.file()).expect("open again");
    let kept = review_of(&store);
    let sent = &kept.threads[0];
    assert_eq!(sent.comments.len(), 1 + replies.len());
    assert_eq!(sent.comments[0].text, "sent");
    assert_eq!(sent.comments[1..], replies[..]);
    // The last reply, number 399, has no mark: it cleared the one reply 398 set.
    assert!(!sent.addressed);
    assert_eq!(kept.threads[1].comments[0].text, last_draft);
    assert_eq!(kept.threads[1].comments[0].round, None);
}

fn refusal_of<T: std::fmt::Debug>(result: Result<T, StoreError>) -> String {
    match result {
        Err(StoreError::Database(sentence)) => sentence,
        other => panic!("not refused with a sentence: {other:?}"),
    }
}

#[test]
fn a_reply_that_the_store_cannot_take_is_refused_and_nothing_changes() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    sent_and_draft(&store);
    // Another review with a second round, whose number review 1 does not have.
    store
        .put_review(&ReviewRecord {
            id: rid(2),
            workspace: WorkspaceId::new(id(2)),
            agent: None,
            reviewed: None,
            threads: vec![],
            rounds: vec![],
            attached: None,
        })
        .expect("put review");
    store
        .put_thread(&rid(2), &thread(20, 200, "of the other review"))
        .expect("put thread");
    store
        .put_round(&rid(2), &round(1), &[cid(200)])
        .expect("put round");
    store
        .put_round(&rid(2), &round(2), &[])
        .expect("put round 2 with no comment");
    let before = review_of(&store);

    let reviewer = Comment {
        author: Author::Reviewer,
        ..reply(500, "a reviewer's comment", 1)
    };
    let no_round = Comment {
        round: None,
        ..reply(502, "no round", 1)
    };
    let reply_of_round_2 = reply(503, "a round the review does not have", 2);
    let taken = reply(100, "an id that is taken", 1);
    let cases = [
        (tid(10), reviewer, "reviewer's"),
        (tid(10), no_round, "no round"),
        (tid(10), reply_of_round_2, "no round 2"),
        (tid(10), taken, &id(100)),
        (tid(99), reply(504, "a thread that is not kept", 1), &id(99)),
    ];
    for (thread, comment, said) in cases {
        for addressed in [true, false] {
            let sentence = refusal_of(store.put_reply(&thread, &comment, addressed));
            assert!(sentence.contains(said), "{sentence}");
            assert_eq!(review_of(&store), before, "after {comment:?}");
        }
    }
    // A thread of another review is not the one the reply's round is looked up in.
    refusal_of(store.put_reply(&tid(20), &reply(505, "round 3", 3), true));
    assert_eq!(
        store.review(&rid(2)).expect("read").expect("kept").threads[0]
            .comments
            .len(),
        1
    );
}

#[test]
fn a_reply_sets_the_addressed_mark_to_what_it_says_and_never_resolves() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    sent_and_draft(&store);
    let addressed = || review_of(&store).threads[0].addressed;
    assert!(!addressed());

    store
        .put_reply(&tid(10), &reply(500, "done", 1), true)
        .expect("reply with the mark");
    assert!(addressed());
    // A reply without the mark takes the earlier mark away.
    store
        .put_reply(&tid(10), &reply(501, "not quite", 1), false)
        .expect("reply without the mark");
    assert!(!addressed());

    // A resolved thread stays resolved, with the mark, and with the reply kept.
    store.resolve_threads(&[tid(10)]).expect("resolve");
    store
        .put_reply(&tid(10), &reply(502, "late", 1), true)
        .expect("reply to a resolved thread");
    let kept = review_of(&store);
    assert!(kept.threads[0].resolved && kept.threads[0].addressed);
    assert_eq!(kept.threads[0].comments.len(), 4);
}

#[test]
fn a_reply_keeps_the_round_it_names_and_not_the_newest_of_the_review() {
    let scratch = Scratch::new();
    {
        let store = Store::open(&scratch.file()).expect("open");
        sent_and_draft(&store);
        // Round 2 exists before the replies, so the newest round is not round 1.
        store
            .put_round(&rid(1), &round(2), &[cid(110)])
            .expect("put round 2");
        store
            .put_reply(&tid(10), &reply(500, "of round 1", 1), false)
            .expect("reply naming round 1");
        store
            .put_reply(&tid(10), &reply(501, "of round 2", 2), false)
            .expect("reply naming round 2");
    }
    // Read from a Store opened anew: what is on the drive.
    let store = Store::open(&scratch.file()).expect("open again");
    let rounds: Vec<Option<u32>> = review_of(&store).threads[0]
        .comments
        .iter()
        .map(|comment| comment.round)
        .collect();
    assert_eq!(rounds, [Some(1), Some(1), Some(2)]);
}

#[test]
fn resolving_several_threads_keeps_all_or_none() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    sent_and_draft(&store);
    let resolved = || {
        review_of(&store)
            .threads
            .iter()
            .map(|t| t.resolved)
            .collect::<Vec<_>>()
    };

    // One that is not kept, after two that are: none of the three is resolved.
    let sentence = refusal_of(store.resolve_threads(&[tid(10), tid(11), tid(99)]));
    assert!(sentence.contains(&id(99)), "{sentence}");
    assert_eq!(resolved(), [false, false]);

    store.resolve_threads(&[]).expect("resolve none");
    assert_eq!(resolved(), [false, false]);
    store
        .resolve_threads(&[tid(10), tid(11)])
        .expect("resolve both");
    assert_eq!(resolved(), [true, true]);
    // Resolving what is resolved is not a fault.
    store.resolve_threads(&[tid(10)]).expect("resolve again");
}

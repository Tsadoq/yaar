//! Component tests: the calls of the Store that keep where a thread sits now, what is
//! attached to the next message, and who is credited with the paths of a recorded state,
//! on a real database file.

mod support;

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use support::Scratch;
use yaar_store::{
    Anchor, AttachedRecord, Author, Comment, CommentId, CreditRecord, NoticeRecord, ReviewId,
    ReviewRecord, Side, Store, StoreError, Thread, ThreadId, ThreadPlace, WorkspaceId,
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

fn workspace(n: u128) -> WorkspaceId {
    WorkspaceId::new(id(n))
}

fn thread(n: u128, first_comment: u128) -> Thread {
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
            text: "text".to_owned(),
            round: None,
        }],
        now: None,
    }
}

/// Two reviews, `1` and `2`, of workspace `1`, with threads `10` and `11` in the first
/// and `20` in the second.
fn two_reviews(store: &Store) {
    for (n, agent) in [(1, Some("session")), (2, None)] {
        store
            .put_review(&ReviewRecord {
                id: rid(n),
                workspace: workspace(1),
                agent: agent.map(str::to_owned),
                reviewed: None,
                threads: vec![],
                rounds: vec![],
                attached: None,
            })
            .expect("put review");
    }
    for (review, n, first) in [(1, 10, 100), (1, 11, 110), (2, 20, 200)] {
        store
            .put_thread(&rid(review), &thread(n, first))
            .expect("put thread");
    }
}

fn review_of(store: &Store, n: u128) -> ReviewRecord {
    store
        .review(&rid(n))
        .expect("read the review")
        .expect("the review is kept")
}

fn refusal_of<T: std::fmt::Debug>(result: Result<T, StoreError>) -> String {
    match result {
        Err(StoreError::Database(sentence)) => sentence,
        other => panic!("not refused with a sentence: {other:?}"),
    }
}

fn place(first: u32, last: u32, notice: Option<NoticeRecord>) -> ThreadPlace {
    ThreadPlace {
        tree: "b".repeat(40),
        path: PathBuf::from(OsStr::from_bytes(b"src/\xffmoved.rs")),
        side: Side::Old,
        first,
        last,
        notice,
    }
}

#[test]
fn places_are_kept_all_or_none_and_none_puts_a_thread_back_on_its_anchor() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    two_reviews(&store);
    let nows = |store: &Store| -> Vec<Option<ThreadPlace>> {
        [
            review_of(store, 1).threads[0].now.clone(),
            review_of(store, 1).threads[1].now.clone(),
            review_of(store, 2).threads[0].now.clone(),
        ]
        .to_vec()
    };
    assert_eq!(nows(&store), [None, None, None]);

    // One thread of the three is not kept: none of the three is placed.
    let sentence = refusal_of(store.set_places(&[
        (tid(10), Some(place(1, 1, None))),
        (tid(20), Some(place(2, 2, None))),
        (tid(99), Some(place(3, 3, None))),
    ]));
    assert!(sentence.contains(&id(99)), "{sentence}");
    assert_eq!(nows(&store), [None, None, None]);
    // A place that no line can be at is refused before anything is kept, and so is one
    // that is refused after a good one.
    refusal_of(store.set_places(&[
        (tid(10), Some(place(1, 1, None))),
        (tid(11), Some(place(0, 1, None))),
    ]));
    refusal_of(store.set_places(&[(tid(10), Some(place(5, 4, None)))]));
    assert_eq!(nows(&store), [None, None, None]);

    // Each thread of the three, in one call, across two reviews.
    store
        .set_places(&[
            (tid(10), Some(place(1, 2, Some(NoticeRecord::Changed)))),
            (tid(11), Some(place(7, 7, Some(NoticeRecord::Deleted)))),
            (tid(20), Some(place(9, 9, None))),
        ])
        .expect("set places");
    let placed = vec![
        Some(place(1, 2, Some(NoticeRecord::Changed))),
        Some(place(7, 7, Some(NoticeRecord::Deleted))),
        Some(place(9, 9, None)),
    ];
    assert_eq!(nows(&store), placed);
    // What the place does not say is as it was: the anchor, the comments.
    assert_eq!(
        review_of(&store, 1).threads[0].anchor,
        thread(10, 100).anchor
    );

    // A later place replaces the earlier, with its notice, and none takes it away;
    // a thread not named keeps its place.
    store
        .set_places(&[(tid(10), Some(place(4, 4, None))), (tid(11), None)])
        .expect("set places again");
    assert_eq!(
        nows(&store),
        [Some(place(4, 4, None)), None, Some(place(9, 9, None))]
    );
    store.set_places(&[]).expect("set no place");

    drop(store);
    let again = Store::open(&scratch.file()).expect("open again");
    assert_eq!(
        nows(&again),
        [Some(place(4, 4, None)), None, Some(place(9, 9, None))]
    );
}

#[test]
fn an_attachment_is_kept_in_place_of_the_last_and_only_for_what_the_review_holds() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    two_reviews(&store);
    let attached = |store: &Store, n| review_of(store, n).attached;
    assert_eq!(attached(&store, 1), None);

    // A thread of another review, one that is not kept, and a review that is not kept.
    let sentence = refusal_of(store.set_attached(&rid(1), Some(&AttachedRecord::Thread(tid(20)))));
    assert!(sentence.contains(&id(20)), "{sentence}");
    refusal_of(store.set_attached(&rid(1), Some(&AttachedRecord::Thread(tid(99)))));
    refusal_of(store.set_attached(&rid(9), Some(&AttachedRecord::Review)));
    refusal_of(store.set_attached(&rid(9), None));
    assert_eq!(attached(&store, 1), None);

    store
        .set_attached(&rid(1), Some(&AttachedRecord::Thread(tid(11))))
        .expect("attach a thread");
    store
        .set_attached(&rid(2), Some(&AttachedRecord::Review))
        .expect("attach a review");
    assert_eq!(attached(&store, 1), Some(AttachedRecord::Thread(tid(11))));
    // What was attached is replaced, not added to, and the other review is its own.
    store
        .set_attached(&rid(1), Some(&AttachedRecord::Review))
        .expect("attach the review");
    assert_eq!(attached(&store, 1), Some(AttachedRecord::Review));
    store
        .set_attached(&rid(1), Some(&AttachedRecord::Thread(tid(10))))
        .expect("attach another thread");

    drop(store);
    let again = Store::open(&scratch.file()).expect("open again");
    assert_eq!(attached(&again, 1), Some(AttachedRecord::Thread(tid(10))));
    assert_eq!(attached(&again, 2), Some(AttachedRecord::Review));
    again.set_attached(&rid(1), None).expect("detach");
    again.set_attached(&rid(1), None).expect("detach again");
    assert_eq!(attached(&again, 1), None);
    assert_eq!(attached(&again, 2), Some(AttachedRecord::Review));
}

#[test]
fn deleting_the_only_draft_of_the_thread_attached_alone_takes_the_attachment_with_it() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    two_reviews(&store);
    store
        .set_attached(&rid(1), Some(&AttachedRecord::Thread(tid(10))))
        .expect("attach a thread");
    store
        .set_attached(&rid(2), Some(&AttachedRecord::Review))
        .expect("attach a review");

    // A thread that stays does not touch the attachment; another thread's draft too.
    store
        .delete_draft(&cid(110))
        .expect("delete the other draft");
    assert_eq!(
        review_of(&store, 1).attached,
        Some(AttachedRecord::Thread(tid(10)))
    );
    store.delete_draft(&cid(100)).expect("delete the draft");
    let kept = review_of(&store, 1);
    assert!(kept.threads.is_empty());
    assert_eq!(kept.attached, None);
    // The attachment of the whole of another review is its own.
    assert_eq!(review_of(&store, 2).attached, Some(AttachedRecord::Review));
}

fn credit(path: &[u8], sessions: &[&str]) -> CreditRecord {
    CreditRecord {
        path: PathBuf::from(OsStr::from_bytes(path)),
        sessions: sessions.iter().map(|s| (*s).to_owned()).collect(),
    }
}

#[test]
fn credits_come_back_by_the_bytes_of_the_paths_with_their_sessions_in_the_order_given() {
    let scratch = Scratch::new();
    let state = "c".repeat(40);
    let given = vec![
        credit(b"src/z.rs", &["s-2", "s-1"]),
        credit(b"b.rs", &[]),
        credit(b"src/\xff odd.rs", &["s-1"]),
        credit(b"B.rs", &["s-3", "s-1", "s-2"]),
    ];
    {
        let store = Store::open(&scratch.file()).expect("open");
        store
            .put_credits(&workspace(1), &state, &given)
            .expect("put credits");
        // Another state and another workspace are their own.
        store
            .put_credits(
                &workspace(1),
                &"d".repeat(40),
                &[credit(b"other.rs", &["s-9"])],
            )
            .expect("put credits");
        store
            .put_credits(&workspace(2), &state, &[credit(b"elsewhere.rs", &[])])
            .expect("put credits");
    }

    let store = Store::open(&scratch.file()).expect("open again");
    // By bytes: `B` is before `b`, and `z` is before the byte 0xff.
    let expected = vec![
        credit(b"B.rs", &["s-3", "s-1", "s-2"]),
        credit(b"b.rs", &[]),
        credit(b"src/z.rs", &["s-2", "s-1"]),
        credit(b"src/\xff odd.rs", &["s-1"]),
    ];
    assert_eq!(
        store.credits_of(&workspace(1), &state).expect("read"),
        expected
    );
    assert_eq!(
        store
            .credits_of(&workspace(1), &"d".repeat(40))
            .expect("read"),
        vec![credit(b"other.rs", &["s-9"])]
    );
    assert_eq!(
        store.credits_of(&workspace(2), &state).expect("read"),
        vec![credit(b"elsewhere.rs", &[])]
    );
    // A state nothing was kept with.
    assert_eq!(
        store
            .credits_of(&workspace(1), &"e".repeat(40))
            .expect("read"),
        vec![]
    );
}

#[test]
fn credits_kept_again_for_a_state_replace_what_was_kept_and_a_name_twice_is_kept_once() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    let state = "c".repeat(40);
    store
        .put_credits(
            &workspace(1),
            &state,
            &[credit(b"a.rs", &["s-1"]), credit(b"b.rs", &[])],
        )
        .expect("put credits");
    store
        .put_credits(
            &workspace(1),
            &state,
            &[
                credit(b"b.rs", &["s-2", "s-2"]),
                credit(b"b.rs", &["s-1"]),
                credit(b"c.rs", &[]),
            ],
        )
        .expect("put credits again");
    assert_eq!(
        store.credits_of(&workspace(1), &state).expect("read"),
        vec![credit(b"b.rs", &["s-2", "s-1"]), credit(b"c.rs", &[])]
    );

    // A state with no change at all is kept as that.
    store
        .put_credits(&workspace(1), &state, &[])
        .expect("put none");
    assert_eq!(
        store.credits_of(&workspace(1), &state).expect("read"),
        vec![]
    );

    // The empty id is how the table says "no session": it cannot be a session's, and
    // nothing of the call is kept.
    store
        .put_credits(&workspace(1), &state, &[credit(b"a.rs", &["s-1"])])
        .expect("put credits");
    refusal_of(store.put_credits(&workspace(1), &state, &[credit(b"z.rs", &["s-1", ""])]));
    assert_eq!(
        store.credits_of(&workspace(1), &state).expect("read"),
        vec![credit(b"a.rs", &["s-1"])]
    );
}

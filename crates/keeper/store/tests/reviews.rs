//! Component tests: the calls of the Store that keep and find reviews, on a real
//! database file.

mod support;

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use support::Scratch;
use yaar_store::{
    Anchor, Author, Comment, CommentId, RepositoryId, ReviewId, ReviewRecord, Round, SentBy, Side,
    Store, StoreError, Thread, ThreadId, ViewedRecord, WorkspaceId,
};

/// An id of 32 hexadecimal digits, from a number.
fn id(n: u128) -> String {
    format!("{n:032x}")
}

fn workspace(n: u128) -> WorkspaceId {
    WorkspaceId::new(id(n))
}

fn review_record(n: u128, workspace: u128, agent: Option<&str>) -> ReviewRecord {
    ReviewRecord {
        id: ReviewId::new(id(n)),
        workspace: self::workspace(workspace),
        agent: agent.map(str::to_owned),
        reviewed: None,
        threads: vec![],
        rounds: vec![],
    }
}

fn anchor(path: &str, side: Side, first: u32, last: u32, text: &str) -> Anchor {
    Anchor {
        tree: "a".repeat(40),
        path: PathBuf::from(path),
        side,
        first,
        last,
        text: text.to_owned(),
    }
}

fn comment(n: u128, author: Author, text: &str, round: Option<u32>) -> Comment {
    Comment {
        id: CommentId::new(id(n)),
        author,
        text: text.to_owned(),
        round,
    }
}

/// A thread with one draft of the reviewer.
fn thread(n: u128, first_comment: u128, text: &str) -> Thread {
    Thread {
        id: ThreadId::new(id(n)),
        anchor: anchor("src/lib.rs", Side::New, 3, 3, "let x = 1;"),
        resolved: false,
        addressed: false,
        comments: vec![comment(first_comment, Author::Reviewer, text, None)],
    }
}

fn round(number: u32) -> Round {
    Round {
        number,
        state: format!("{number:040x}"),
        sent_by: SentBy::Copied,
    }
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

/// A review, `1` of workspace `1`, with a thread `10` holding a draft `100`.
fn with_one_draft(store: &Store) {
    store
        .put_review(&review_record(1, 1, None))
        .expect("put review");
    store
        .put_thread(&rid(1), &thread(10, 100, "first"))
        .expect("put thread");
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

/// How many bytes the write-ahead file holds; none when it is not there.
fn wal_len(scratch: &Scratch) -> u64 {
    std::fs::metadata(scratch.0.join("store.db-wal")).map_or(0, |m| m.len())
}

#[test]
fn every_call_keeps_what_it_is_given_and_a_store_opened_later_finds_it_again() {
    let scratch = Scratch::new();
    let odd = PathBuf::from(OsStr::from_bytes(b"src/\xff odd\n'name\".rs"));
    let expected;
    {
        let store = Store::open(&scratch.file()).expect("open");
        // Ids that no sorting puts in the order they are kept in.
        store
            .put_review(&review_record(9, 1, None))
            .expect("put review");
        store
            .put_review(&review_record(5, 1, Some("session-b")))
            .expect("put review");
        store
            .put_review(&review_record(7, 2, None))
            .expect("put review of another workspace");

        let mut removed = thread(30, 300, "on the old side");
        removed.anchor = Anchor {
            tree: "c".repeat(40),
            path: odd.clone(),
            side: Side::Old,
            first: 7,
            last: 9,
            text: "a\nb\nc".to_owned(),
        };
        store
            .put_thread(&rid(9), &thread(20, 200, "on the new side\nin two lines"))
            .expect("put thread");
        store.put_thread(&rid(9), &removed).expect("put thread");
        store
            .put_thread(&rid(5), &thread(40, 400, "of the agent's review"))
            .expect("put thread");
        // Comments of two threads, kept in turn: each thread finds its own, in order.
        store
            .put_comment(
                &tid(20),
                &comment(210, Author::Reviewer, "first reply", None),
            )
            .expect("put comment");
        store
            .put_comment(
                &tid(30),
                &comment(310, Author::Agent, "the agent answers", Some(1)),
            )
            .expect("put comment");
        store
            .put_comment(
                &tid(20),
                &comment(205, Author::Reviewer, "second reply", None),
            )
            .expect("put comment");

        store
            .save_draft(&cid(200), "on the new side, edited")
            .expect("save draft");
        store
            .put_round(&rid(9), &round(1), &[cid(200), cid(300)])
            .expect("put round");
        store
            .put_round(&rid(9), &round(2), &[cid(210)])
            .expect("put round");
        store.set_resolved(&tid(30), true).expect("set resolved");
        store
            .set_reviewed(&rid(9), &"d".repeat(40))
            .expect("set reviewed");

        expected = vec![
            ReviewRecord {
                id: rid(9),
                workspace: workspace(1),
                agent: None,
                reviewed: Some("d".repeat(40)),
                threads: vec![
                    Thread {
                        comments: vec![
                            comment(200, Author::Reviewer, "on the new side, edited", Some(1)),
                            comment(210, Author::Reviewer, "first reply", Some(2)),
                            comment(205, Author::Reviewer, "second reply", None),
                        ],
                        ..thread(20, 200, "")
                    },
                    Thread {
                        resolved: true,
                        comments: vec![
                            comment(300, Author::Reviewer, "on the old side", Some(1)),
                            comment(310, Author::Agent, "the agent answers", Some(1)),
                        ],
                        ..removed
                    },
                ],
                rounds: vec![round(1), round(2)],
            },
            ReviewRecord {
                id: rid(5),
                workspace: workspace(1),
                agent: Some("session-b".to_owned()),
                reviewed: None,
                threads: vec![thread(40, 400, "of the agent's review")],
                rounds: vec![],
            },
        ];
        assert_eq!(store.reviews_of(&workspace(1)).expect("reviews"), expected);
    }

    for _ in 0..2 {
        // A second Store on the same file, as after a restart; the first one is gone.
        let store = Store::open(&scratch.file()).expect("open again");
        let kept = store.reviews_of(&workspace(1)).expect("reviews");
        assert_eq!(kept, expected);
        assert_eq!(review_of(&store, 9), expected[0]);
        assert_eq!(review_of(&store, 5), expected[1]);
        // The reviews of another workspace stay out of it, and are whole themselves.
        assert_eq!(
            store.reviews_of(&workspace(2)).expect("reviews"),
            vec![review_record(7, 2, None)]
        );
        assert_eq!(store.reviews_of(&workspace(3)).expect("reviews"), vec![]);
        assert_eq!(store.review(&rid(99)).expect("review"), None);
    }
}

#[test]
fn a_second_review_of_no_agent_for_a_workspace_is_refused_and_the_first_is_unchanged() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    with_one_draft(&store);
    let before = review_of(&store, 1);

    let sentence = refusal_of(store.put_review(&review_record(2, 1, None)));
    assert!(
        sentence.contains(&id(1)) && sentence.contains("review of no agent already"),
        "{sentence}"
    );
    assert_eq!(review_of(&store, 1), before);
    assert_eq!(store.review(&rid(2)).expect("review"), None);

    // A review of an agent beside it is kept, and so is one of no agent in another
    // workspace; two of one agent are no concern of this rule.
    store
        .put_review(&review_record(3, 1, Some("session")))
        .expect("a review of an agent");
    store
        .put_review(&review_record(4, 1, Some("session")))
        .expect("another of the same agent");
    store
        .put_review(&review_record(5, 2, None))
        .expect("a review of no agent of another workspace");
    assert_eq!(store.reviews_of(&workspace(1)).expect("reviews").len(), 3);
    assert_eq!(review_of(&store, 1), before);
}

#[test]
fn a_review_whose_id_is_taken_or_which_comes_with_threads_is_refused_and_nothing_changes() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    with_one_draft(&store);
    let before = review_of(&store, 1);

    // The id is taken, in another workspace and as a review of an agent.
    let mut taken = review_record(1, 2, Some("session"));
    taken.reviewed = Some("e".repeat(40));
    let sentence = refusal_of(store.put_review(&taken));
    assert!(sentence.contains(&id(1)), "{sentence}");
    assert_eq!(review_of(&store, 1), before);
    assert_eq!(store.reviews_of(&workspace(2)).expect("reviews"), vec![]);

    let mut with_a_thread = review_record(2, 2, None);
    with_a_thread.threads = vec![thread(11, 101, "smuggled")];
    refusal_of(store.put_review(&with_a_thread));
    let mut with_a_round = review_record(2, 2, None);
    with_a_round.rounds = vec![round(1)];
    refusal_of(store.put_review(&with_a_round));
    assert_eq!(store.review(&rid(2)).expect("review"), None);
}

#[test]
fn a_thread_or_a_comment_that_the_store_cannot_take_is_refused_and_nothing_changes() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    with_one_draft(&store);
    store
        .put_review(&review_record(2, 1, Some("session")))
        .expect("put review");
    let before = (review_of(&store, 1), review_of(&store, 2));

    // A thread of a review the Store does not hold.
    let sentence = refusal_of(store.put_thread(&rid(99), &thread(11, 101, "x")));
    assert!(sentence.contains(&id(99)), "{sentence}");
    // A thread id that is taken, in the same review and in another one.
    refusal_of(store.put_thread(&rid(1), &thread(10, 101, "x")));
    refusal_of(store.put_thread(&rid(2), &thread(10, 101, "x")));
    // A comment id that is taken, by the first comment of another thread.
    refusal_of(store.put_thread(&rid(1), &thread(11, 100, "x")));
    // Lines that count from 0, or end before they start.
    let mut from_zero = thread(11, 101, "x");
    from_zero.anchor.first = 0;
    refusal_of(store.put_thread(&rid(1), &from_zero));
    let mut backwards = thread(11, 101, "x");
    (backwards.anchor.first, backwards.anchor.last) = (5, 4);
    refusal_of(store.put_thread(&rid(1), &backwards));
    // Not one draft of the reviewer: none, two, a sent one, the agent's.
    let mut none = thread(11, 101, "x");
    none.comments.clear();
    refusal_of(store.put_thread(&rid(1), &none));
    let mut two = thread(11, 101, "x");
    two.comments.push(comment(102, Author::Reviewer, "y", None));
    refusal_of(store.put_thread(&rid(1), &two));
    let mut sent = thread(11, 101, "x");
    sent.comments[0].round = Some(1);
    refusal_of(store.put_thread(&rid(1), &sent));
    let mut agent = thread(11, 101, "x");
    agent.comments[0].author = Author::Agent;
    refusal_of(store.put_thread(&rid(1), &agent));

    // A reply to a thread that is not kept, and a comment id that is taken.
    let sentence =
        refusal_of(store.put_comment(&tid(99), &comment(101, Author::Reviewer, "x", None)));
    assert!(sentence.contains(&id(99)), "{sentence}");
    refusal_of(store.put_comment(&tid(10), &comment(100, Author::Reviewer, "x", None)));

    assert_eq!((review_of(&store, 1), review_of(&store, 2)), before);
}

#[test]
fn a_draft_is_saved_whole_each_time_and_only_a_draft_of_the_reviewer_is() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    with_one_draft(&store);
    store
        .put_comment(&tid(10), &comment(101, Author::Reviewer, "reply", None))
        .expect("put comment");
    // An agent's reply with no round, as slice 6 may leave it: not a draft.
    store
        .put_comment(&tid(10), &comment(102, Author::Agent, "agent's", None))
        .expect("put comment");
    store
        .put_round(&rid(1), &round(1), &[cid(100)])
        .expect("send the first");

    for text in ["r", "re", "re\nply", "", "ünïcode ✓"] {
        store.save_draft(&cid(101), text).expect("save draft");
        let kept = review_of(&store, 1);
        assert_eq!(kept.threads[0].comments[1].text, text);
        assert_eq!(kept.threads[0].comments[1].round, None);
    }
    let before = review_of(&store, 1);

    // Sent, the agent's with no round, and one the Store does not hold.
    let sentence = refusal_of(store.save_draft(&cid(100), "late"));
    assert!(
        sentence.contains(&id(100)) && sentence.contains("round 1"),
        "{sentence}"
    );
    let sentence = refusal_of(store.save_draft(&cid(102), "mine now"));
    assert!(sentence.contains(&id(102)), "{sentence}");
    let sentence = refusal_of(store.save_draft(&cid(999), "nowhere"));
    assert!(sentence.contains("not kept"), "{sentence}");
    assert_eq!(review_of(&store, 1), before);
    // The refusals left the store working as it did.
    store
        .save_draft(&cid(101), "after the refusals")
        .expect("save draft");
}

#[test]
fn deleting_a_draft_takes_its_thread_with_it_only_when_it_was_the_last_comment() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    with_one_draft(&store);
    store
        .put_round(&rid(1), &round(1), &[cid(100)])
        .expect("send");
    store
        .put_comment(&tid(10), &comment(101, Author::Reviewer, "reply", None))
        .expect("put comment");
    store
        .put_comment(&tid(10), &comment(102, Author::Agent, "agent's", None))
        .expect("put comment");
    store
        .put_thread(&rid(1), &thread(11, 110, "alone"))
        .expect("put thread");
    let before = review_of(&store, 1);

    // Refused: sent, the agent's with no round, not kept. Nothing changes.
    refusal_of(store.delete_draft(&cid(100)));
    refusal_of(store.delete_draft(&cid(102)));
    let sentence = refusal_of(store.delete_draft(&cid(999)));
    assert!(sentence.contains("not kept"), "{sentence}");
    assert_eq!(review_of(&store, 1), before);

    // A draft reply goes and its thread stays, with the others.
    store.delete_draft(&cid(101)).expect("delete the reply");
    let kept = review_of(&store, 1);
    let ids: Vec<&CommentId> = kept.threads[0].comments.iter().map(|c| &c.id).collect();
    assert_eq!(ids, [&cid(100), &cid(102)]);
    assert_eq!(kept.threads.len(), 2);

    // The only comment of a thread goes and takes the thread.
    store
        .delete_draft(&cid(110))
        .expect("delete the only draft");
    let kept = review_of(&store, 1);
    assert_eq!(kept.threads.len(), 1);
    assert_eq!(kept.threads[0].id, tid(10));
    // And what was deleted is not there to save, or to delete again.
    refusal_of(store.save_draft(&cid(110), "back"));
    refusal_of(store.delete_draft(&cid(110)));
}

#[test]
fn a_round_that_is_not_the_next_or_names_a_comment_that_is_not_a_draft_of_the_review_changes_nothing()
 {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    with_one_draft(&store);
    store
        .put_thread(&rid(1), &thread(11, 110, "second"))
        .expect("put thread");
    store
        .put_comment(&tid(11), &comment(111, Author::Agent, "agent's", None))
        .expect("put comment");
    store
        .put_review(&review_record(2, 2, None))
        .expect("put review");
    store
        .put_thread(&rid(2), &thread(20, 200, "of the other review"))
        .expect("put thread");
    let before = (review_of(&store, 1), review_of(&store, 2));
    let unchanged = |store: &Store| {
        assert_eq!((review_of(store, 1), review_of(store, 2)), before);
    };

    // A number that is not 1: 0 and 2.
    for number in [0, 2] {
        let sentence = refusal_of(store.put_round(&rid(1), &round(number), &[cid(100)]));
        assert!(sentence.contains("next"), "{sentence}");
        unchanged(&store);
    }
    // A review that is not kept.
    refusal_of(store.put_round(&rid(99), &round(1), &[cid(100)]));
    // A comment of another review, one that is not kept, the agent's with no round,
    // and a comment named twice; each after a good one, which must not stay sent.
    for bad in [cid(200), cid(999), cid(111), cid(100)] {
        refusal_of(store.put_round(&rid(1), &round(1), &[cid(100), bad]));
        unchanged(&store);
    }

    // The same round is kept once it is right, and then the next number is 2.
    store
        .put_round(&rid(1), &round(1), &[cid(100)])
        .expect("put round");
    let sentence = refusal_of(store.put_round(&rid(1), &round(1), &[cid(110)]));
    assert!(sentence.contains("next"), "{sentence}");
    let sentence = refusal_of(store.put_round(&rid(1), &round(2), &[cid(100)]));
    assert!(sentence.contains(&id(100)), "{sentence}");
    let kept = review_of(&store, 1);
    assert_eq!(kept.rounds, vec![round(1)]);
    assert_eq!(kept.threads[0].comments[0].round, Some(1));
    assert_eq!(kept.threads[1].comments[0].round, None);
    assert_eq!(kept.threads[1].comments[1].round, None);
    store
        .put_round(&rid(1), &round(2), &[cid(110)])
        .expect("put round 2");
    assert_eq!(review_of(&store, 1).rounds, vec![round(1), round(2)]);
    assert_eq!(review_of(&store, 2), before.1);
}

#[test]
fn resolving_and_the_reviewed_state_are_kept_and_refused_for_what_is_not_kept() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    with_one_draft(&store);

    store.set_resolved(&tid(10), true).expect("resolve");
    assert!(review_of(&store, 1).threads[0].resolved);
    store.set_resolved(&tid(10), false).expect("unresolve");
    assert!(!review_of(&store, 1).threads[0].resolved);
    assert_eq!(review_of(&store, 1).reviewed, None);
    store.set_reviewed(&rid(1), &"1".repeat(40)).expect("mark");
    store
        .set_reviewed(&rid(1), &"2".repeat(40))
        .expect("mark again");
    assert_eq!(review_of(&store, 1).reviewed, Some("2".repeat(40)));

    refusal_of(store.set_resolved(&tid(99), true));
    refusal_of(store.set_reviewed(&rid(99), &"3".repeat(40)));
    assert_eq!(review_of(&store, 1).reviewed, Some("2".repeat(40)));
}

#[test]
fn the_files_marked_viewed_are_kept_for_each_workspace_by_the_bytes_of_their_paths() {
    let scratch = Scratch::new();
    let odd = Path::new(OsStr::from_bytes(b"a/\xff.rs"));
    {
        let store = Store::open(&scratch.file()).expect("open");
        assert_eq!(store.viewed(&workspace(1)).expect("viewed"), vec![]);
        for (path, mark) in [
            (Path::new("b.rs"), "one"),
            (Path::new("B.rs"), "upper"),
            (odd, "odd"),
            (Path::new("a/z.rs"), "deep"),
        ] {
            store
                .set_viewed(&workspace(1), path, Some(mark))
                .expect("mark");
        }
        store
            .set_viewed(&workspace(2), Path::new("b.rs"), Some("of another"))
            .expect("mark");
        // Marked again, the content is the new one; and a mark taken away is gone.
        store
            .set_viewed(&workspace(1), Path::new("b.rs"), Some("two"))
            .expect("mark again");
        store
            .set_viewed(&workspace(1), Path::new("a/z.rs"), None)
            .expect("take away");
        // Taking away what is not marked is no failure.
        store
            .set_viewed(&workspace(1), Path::new("never.rs"), None)
            .expect("take away nothing");
    }
    let store = Store::open(&scratch.file()).expect("open again");
    let viewed = |path: &Path, content: &str| ViewedRecord {
        path: path.to_owned(),
        content: content.to_owned(),
    };
    assert_eq!(
        store.viewed(&workspace(1)).expect("viewed"),
        vec![
            viewed(Path::new("B.rs"), "upper"),
            viewed(odd, "odd"),
            viewed(Path::new("b.rs"), "two"),
        ]
    );
    assert_eq!(
        store.viewed(&workspace(2)).expect("viewed"),
        vec![viewed(Path::new("b.rs"), "of another")]
    );
}

#[test]
fn the_workspaces_with_threads_come_in_the_order_the_workspaces_were_kept() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    assert_eq!(store.workspaces_with_threads().expect("listed"), vec![]);
    store
        .put_repository(&yaar_store::RepositoryRecord {
            id: RepositoryId::new("r"),
            git_dir: PathBuf::from("/r/.git"),
        })
        .expect("put repository");
    // Workspaces kept as 3, 1, 2; the reviews are put in another order, and a workspace
    // that the table does not hold has its reviews among them.
    for n in [3, 1, 2] {
        store
            .put_workspace(&workspace(n), &RepositoryId::new("r"), Path::new("/r"))
            .expect("put workspace");
    }
    for (review, of, agent) in [
        (10, 2, None),
        (11, 1, None),
        (12, 3, None),
        (13, 3, Some("session")),
        (14, 8, None),
        (15, 7, None),
        (16, 4, None),
    ] {
        store
            .put_review(&review_record(review, of, agent))
            .expect("put review");
    }
    // Reviews of 4 holds no thread; the others hold one each, workspace 3 in two reviews.
    for (review, n) in [(15, 0), (10, 1), (14, 2), (13, 3), (12, 4), (11, 5)] {
        store
            .put_thread(&rid(review), &thread(100 + n, 1000 + n, "x"))
            .expect("put thread");
    }

    // 8's review was put before 7's, and neither workspace is in the table.
    assert_eq!(
        store.workspaces_with_threads().expect("listed"),
        vec![
            workspace(3),
            workspace(1),
            workspace(2),
            workspace(8),
            workspace(7)
        ]
    );

    // The last draft deleted takes the thread, and the workspace leaves the list; the
    // other of a workspace's reviews still holds one.
    store.delete_draft(&cid(1001)).expect("delete");
    store.delete_draft(&cid(1002)).expect("delete");
    assert_eq!(
        store.workspaces_with_threads().expect("listed"),
        vec![workspace(3), workspace(1), workspace(7)]
    );
    store.delete_draft(&cid(1003)).expect("delete");
    assert_eq!(
        store.workspaces_with_threads().expect("listed"),
        vec![workspace(3), workspace(1), workspace(7)]
    );
}

#[test]
fn a_flush_moves_the_saved_drafts_out_of_the_write_ahead_file_at_once() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    with_one_draft(&store);
    store.save_draft(&cid(100), "typed").expect("save draft");
    assert!(wal_len(&scratch) > 0, "a save does not flush by itself");

    store.flush().expect("flush");
    assert_eq!(wal_len(&scratch), 0);
    assert_eq!(review_of(&store, 1).threads[0].comments[0].text, "typed");
    // A flush with nothing to do is no failure.
    store.flush().expect("flush again");
}

#[test]
fn the_writer_flushes_by_itself_once_the_typing_pauses() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    with_one_draft(&store);
    store.save_draft(&cid(100), "typed").expect("save draft");
    assert!(wal_len(&scratch) > 0, "a save does not flush by itself");

    // Waiting for a fact, with a deadline far past the pause the writer allows itself.
    let deadline = Instant::now() + Duration::from_secs(30);
    while wal_len(&scratch) > 0 {
        assert!(Instant::now() < deadline, "the writer never flushed");
        std::thread::sleep(Duration::from_millis(20));
    }
    // It goes on working after a flush of its own.
    store
        .save_draft(&cid(100), "typed more")
        .expect("save draft");
    assert_eq!(
        review_of(&store, 1).threads[0].comments[0].text,
        "typed more"
    );
}

#[test]
fn the_last_draft_saved_is_in_the_file_alone_when_the_last_clone_is_dropped() {
    let scratch = Scratch::new();
    {
        let store = Store::open(&scratch.file()).expect("open");
        let clone = store.clone();
        with_one_draft(&store);
        drop(store);
        clone
            .save_draft(&cid(100), "typed last")
            .expect("save draft");
    }
    let elsewhere = Scratch::new();
    std::fs::copy(scratch.file(), elsewhere.file()).expect("copy the file");

    let store = Store::open(&elsewhere.file()).expect("open the copy");
    assert_eq!(
        review_of(&store, 1).threads[0].comments[0].text,
        "typed last"
    );
}

#[test]
fn a_send_is_in_the_database_file_itself_before_the_call_returns() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    with_one_draft(&store);
    store.save_draft(&cid(100), "typed").expect("save draft");
    store
        .put_round(&rid(1), &round(1), &[cid(100)])
        .expect("put round");
    assert_eq!(wal_len(&scratch), 0);

    // The file alone, copied while the Store is open and without its write-ahead file,
    // holds the sent comment: a write-ahead file damaged later costs nothing sent.
    let elsewhere = Scratch::new();
    std::fs::copy(scratch.file(), elsewhere.file()).expect("copy the file");
    let copy = Store::open(&elsewhere.file()).expect("open the copy");
    let kept = review_of(&copy, 1);
    assert_eq!(kept.rounds, vec![round(1)]);
    assert_eq!(
        kept.threads[0].comments,
        vec![comment(100, Author::Reviewer, "typed", Some(1))]
    );
}

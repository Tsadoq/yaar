mod support;

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use support::Scratch;
use yaar_store::{
    Anchor, Author, Comment, CommentId, FLUSH_AFTER, RepositoryId, ReviewId, ReviewRecord, Round,
    SentBy, Side, Store, StoreError, Thread, ThreadId, ViewedRecord, WorkspaceId,
};

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
        attached: None,
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

fn thread(n: u128, first_comment: u128, text: &str) -> Thread {
    Thread {
        id: ThreadId::new(id(n)),
        anchor: anchor("src/lib.rs", Side::New, 3, 3, "let x = 1;"),
        resolved: false,
        addressed: false,
        comments: vec![comment(first_comment, Author::Reviewer, text, None)],
        now: None,
    }
}

fn round(number: u32) -> Round {
    Round {
        number,
        state: Some(format!("{number:040x}")),
        sent_by: SentBy::Copied,
        edited: false,
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

fn wal_len(scratch: &Scratch) -> u64 {
    std::fs::metadata(scratch.0.join("store.db-wal")).map_or(0, |m| m.len())
}

/// A runner that stalls longer than the writer waits has the writer flush on its own, so check only
/// while the writer's pause has not run out.
fn assert_not_flushed_yet(scratch: &Scratch, saved_at: Instant) {
    let held = wal_len(scratch);
    if saved_at.elapsed() < FLUSH_AFTER {
        assert!(held > 0, "a save does not flush by itself");
    }
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
                attached: None,
            },
            ReviewRecord {
                id: rid(5),
                workspace: workspace(1),
                agent: Some("session-b".to_owned()),
                reviewed: None,
                threads: vec![thread(40, 400, "of the agent's review")],
                rounds: vec![],
                attached: None,
            },
        ];
        assert_eq!(store.reviews_of(&workspace(1)).expect("reviews"), expected);
    }

    for _ in 0..2 {
        let store = Store::open(&scratch.file()).expect("open again");
        let kept = store.reviews_of(&workspace(1)).expect("reviews");
        assert_eq!(kept, expected);
        assert_eq!(review_of(&store, 9), expected[0]);
        assert_eq!(review_of(&store, 5), expected[1]);
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

    let sentence = refusal_of(store.put_thread(&rid(99), &thread(11, 101, "x")));
    assert!(sentence.contains(&id(99)), "{sentence}");
    refusal_of(store.put_thread(&rid(1), &thread(10, 101, "x")));
    refusal_of(store.put_thread(&rid(2), &thread(10, 101, "x")));
    refusal_of(store.put_thread(&rid(1), &thread(11, 100, "x")));
    let mut from_zero = thread(11, 101, "x");
    from_zero.anchor.first = 0;
    refusal_of(store.put_thread(&rid(1), &from_zero));
    let mut backwards = thread(11, 101, "x");
    (backwards.anchor.first, backwards.anchor.last) = (5, 4);
    refusal_of(store.put_thread(&rid(1), &backwards));
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

    refusal_of(store.delete_draft(&cid(100)));
    refusal_of(store.delete_draft(&cid(102)));
    let sentence = refusal_of(store.delete_draft(&cid(999)));
    assert!(sentence.contains("not kept"), "{sentence}");
    assert_eq!(review_of(&store, 1), before);

    store.delete_draft(&cid(101)).expect("delete the reply");
    let kept = review_of(&store, 1);
    let ids: Vec<&CommentId> = kept.threads[0].comments.iter().map(|c| &c.id).collect();
    assert_eq!(ids, [&cid(100), &cid(102)]);
    assert_eq!(kept.threads.len(), 2);

    store
        .delete_draft(&cid(110))
        .expect("delete the only draft");
    let kept = review_of(&store, 1);
    assert_eq!(kept.threads.len(), 1);
    assert_eq!(kept.threads[0].id, tid(10));
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

    for number in [0, 2] {
        let sentence = refusal_of(store.put_round(&rid(1), &round(number), &[cid(100)]));
        assert!(sentence.contains("next"), "{sentence}");
        unchanged(&store);
    }
    refusal_of(store.put_round(&rid(99), &round(1), &[cid(100)]));
    // Each bad comment comes after a good one, which must not stay sent.
    for bad in [cid(200), cid(999), cid(111), cid(100)] {
        refusal_of(store.put_round(&rid(1), &round(1), &[cid(100), bad]));
        unchanged(&store);
    }

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
        store
            .set_viewed(&workspace(1), Path::new("b.rs"), Some("two"))
            .expect("mark again");
        store
            .set_viewed(&workspace(1), Path::new("a/z.rs"), None)
            .expect("take away");
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
    // Workspaces kept as 3, 1, 2, reviews put in another order; workspaces 4, 7 and 8 have
    // reviews but are not in the table.
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
    for (review, n) in [(15, 0), (10, 1), (14, 2), (13, 3), (12, 4), (11, 5)] {
        store
            .put_thread(&rid(review), &thread(100 + n, 1000 + n, "x"))
            .expect("put thread");
    }

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
    let saved_at = Instant::now();
    store.save_draft(&cid(100), "typed").expect("save draft");
    assert_not_flushed_yet(&scratch, saved_at);

    store.flush().expect("flush");
    assert_eq!(wal_len(&scratch), 0);
    assert_eq!(review_of(&store, 1).threads[0].comments[0].text, "typed");
    store.flush().expect("flush again");
}

#[test]
fn the_writer_flushes_by_itself_once_the_typing_pauses() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    with_one_draft(&store);
    let saved_at = Instant::now();
    store.save_draft(&cid(100), "typed").expect("save draft");
    assert_not_flushed_yet(&scratch, saved_at);

    // Deadline far past the pause the writer allows itself.
    let deadline = Instant::now() + Duration::from_secs(30);
    while wal_len(&scratch) > 0 {
        assert!(Instant::now() < deadline, "the writer never flushed");
        std::thread::sleep(Duration::from_millis(20));
    }
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

    // The file alone, copied without its write-ahead file, holds the sent comment.
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

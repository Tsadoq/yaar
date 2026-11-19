//! Component tests: everything the Store holds of reviews and of turns reads back as it
//! was after the database is closed and opened again, on a real database file.

mod support;

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use support::Scratch;
use yaar_store::{
    Anchor, AttachedRecord, Author, Comment, CommentId, NoticeRecord, ReviewId, ReviewRecord,
    Round, SentBy, SessionId, SessionRecord, Side, StepRecord, Store, Thread, ThreadId,
    ThreadPlace, TurnEndRecord, TurnId, TurnRecord, WorkspaceId,
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

fn workspace() -> WorkspaceId {
    WorkspaceId::new(id(1))
}

fn anchor(path: PathBuf, side: Side, first: u32, last: u32) -> Anchor {
    Anchor {
        tree: "a".repeat(40),
        path,
        side,
        first,
        last,
        text: "let x = 1;".to_owned(),
    }
}

fn comment(n: u128, author: Author, text: &str, round: Option<u32>) -> Comment {
    Comment {
        id: cid(n),
        author,
        text: text.to_owned(),
        round,
    }
}

/// A thread `n` with one draft of the reviewer, `first_comment`.
fn thread(n: u128, first_comment: u128, text: &str, anchor: Anchor) -> Thread {
    Thread {
        id: tid(n),
        anchor,
        resolved: false,
        addressed: false,
        comments: vec![comment(first_comment, Author::Reviewer, text, None)],
        now: None,
    }
}

fn review(n: u128, agent: Option<&str>) -> ReviewRecord {
    ReviewRecord {
        id: rid(n),
        workspace: workspace(),
        agent: agent.map(str::to_owned),
        reviewed: None,
        threads: vec![],
        rounds: vec![],
        attached: None,
    }
}

/// What the Store holds, read through its calls.
#[derive(Debug, PartialEq)]
struct Everything {
    reviews: Vec<ReviewRecord>,
    turns: Vec<TurnRecord>,
    turn_with_steps: Vec<(TurnRecord, Vec<StepRecord>)>,
    session: Option<SessionRecord>,
}

fn read_everything(store: &Store) -> Everything {
    let turns = store.turns_of(&workspace()).expect("turns");
    Everything {
        reviews: store.reviews_of(&workspace()).expect("reviews"),
        turn_with_steps: turns
            .iter()
            .map(|turn| {
                store
                    .turn(&turn.id)
                    .expect("read the turn")
                    .expect("the turn is kept")
            })
            .collect(),
        turns,
        session: store.session(&SessionId::new(id(50))).expect("session"),
    }
}

#[test]
fn after_the_database_is_closed_and_opened_again_every_review_reads_back_equal_with_its_turns() {
    let scratch = Scratch::new();
    // A path that is not text, and its old side.
    let odd = PathBuf::from(OsStr::from_bytes(b"src/\xff odd\n'name\".rs"));
    let state = |c: char| c.to_string().repeat(40);

    let expected;
    {
        let store = Store::open(&scratch.file()).expect("open");
        store
            .put_review(&review(1, Some("session-a")))
            .expect("put review");
        store.put_review(&review(2, None)).expect("put review");

        // Review 1: a thread whose lines were rewritten and which the agent answered
        // (10), a draft (11) that is attached alone, a resolved thread whose lines were
        // deleted (12), and a thread of round 2 that stays on its anchor (13). The draft
        // (11) is placed with no notice.
        let rewritten = anchor(odd.clone(), Side::Old, 4, 6);
        let plain = anchor(PathBuf::from("src/lib.rs"), Side::New, 3, 3);
        for (n, first, text, anchor) in [
            (10, 100, "fix this", rewritten),
            (11, 110, "typed, not sent", plain.clone()),
            (12, 120, "drop this", plain.clone()),
            (13, 130, "and this", plain.clone()),
        ] {
            store
                .put_thread(&rid(1), &thread(n, first, text, anchor))
                .expect("put thread");
        }
        // The first round is kept with no state, which is recorded after.
        store
            .put_round(&rid(1), &delivered(1, None), &[cid(100), cid(120)])
            .expect("put round 1");
        store
            .set_round_state(&rid(1), 1, &state('b'))
            .expect("set the state of round 1");
        store
            .set_round_edited(&rid(1), 1)
            .expect("set the edit mark");
        store
            .put_reply(
                &tid(10),
                &comment(101, Author::Agent, "done\n`ok`", Some(1)),
                true,
            )
            .expect("put reply");
        store.resolve_threads(&[tid(12)]).expect("resolve");
        store
            .put_round(&rid(1), &delivered(2, None), &[cid(130)])
            .expect("put round 2");
        store
            .set_reviewed(&rid(1), &state('c'))
            .expect("set reviewed");
        store
            .set_places(&[
                (
                    tid(10),
                    Some(ThreadPlace {
                        tree: state('d'),
                        path: odd.clone(),
                        side: Side::New,
                        first: 8,
                        last: 9,
                        notice: Some(NoticeRecord::Changed),
                    }),
                ),
                (
                    tid(11),
                    Some(ThreadPlace {
                        tree: state('d'),
                        path: PathBuf::from("src/lib.rs"),
                        side: Side::New,
                        first: 5,
                        last: 5,
                        notice: None,
                    }),
                ),
                (
                    tid(12),
                    Some(ThreadPlace {
                        tree: state('e'),
                        path: PathBuf::from("src/lib.rs"),
                        side: Side::Old,
                        first: 3,
                        last: 3,
                        notice: Some(NoticeRecord::Deleted),
                    }),
                ),
            ])
            .expect("set places");
        store
            .set_attached(&rid(1), Some(&AttachedRecord::Thread(tid(11))))
            .expect("attach a thread");

        // Review 2: attached whole, with a draft.
        store
            .put_thread(&rid(2), &thread(20, 200, "of no agent", plain))
            .expect("put thread");
        store
            .set_attached(&rid(2), Some(&AttachedRecord::Review))
            .expect("attach the review");

        // A session with two turns: the first ended, with an edit that names its lines.
        let session = SessionRecord {
            id: SessionId::new(id(50)),
            workspace: workspace(),
            agent: "claude".to_owned(),
            cli_session: "cli-50".to_owned(),
            terminal: Some("t1".to_owned()),
            number: 1,
            running: true,
            ended: None,
            ended_by: None,
        };
        store.put_session(&session).expect("put session");
        let turn = |n: u128, number| TurnRecord {
            id: TurnId::new(id(n)),
            session: session.id.clone(),
            number,
            ended: None,
            start: None,
            state: None,
        };
        store.put_turn(&turn(60, 1)).expect("put turn");
        store.put_turn(&turn(61, 2)).expect("put turn");
        store
            .set_turn_start(&TurnId::new(id(60)), &state('f'))
            .expect("set the start");
        store
            .put_step(&TurnId::new(id(60)), 1, &StepRecord::Read(odd.clone()))
            .expect("put step");
        store
            .put_step(
                &TurnId::new(id(60)),
                2,
                &StepRecord::Edit(odd.clone(), Some(vec![3..=4, 9..=9])),
            )
            .expect("put step");
        store
            .put_step(
                &TurnId::new(id(61)),
                1,
                &StepRecord::Command("cargo test".to_owned()),
            )
            .expect("put step");
        store
            .end_turn(
                &TurnId::new(id(60)),
                TurnEndRecord::Finished,
                Some(&state('9')),
            )
            .expect("end turn");

        // The records the test built, field for field.
        let sent = |n, text: &str, round| comment(n, Author::Reviewer, text, round);
        expected = Everything {
            reviews: vec![
                ReviewRecord {
                    reviewed: Some(state('c')),
                    threads: vec![
                        Thread {
                            addressed: true,
                            comments: vec![
                                sent(100, "fix this", Some(1)),
                                comment(101, Author::Agent, "done\n`ok`", Some(1)),
                            ],
                            now: Some(ThreadPlace {
                                tree: state('d'),
                                path: odd.clone(),
                                side: Side::New,
                                first: 8,
                                last: 9,
                                notice: Some(NoticeRecord::Changed),
                            }),
                            ..thread(10, 100, "", anchor(odd.clone(), Side::Old, 4, 6))
                        },
                        Thread {
                            now: Some(ThreadPlace {
                                tree: state('d'),
                                path: PathBuf::from("src/lib.rs"),
                                side: Side::New,
                                first: 5,
                                last: 5,
                                notice: None,
                            }),
                            ..thread(
                                11,
                                110,
                                "typed, not sent",
                                anchor(PathBuf::from("src/lib.rs"), Side::New, 3, 3),
                            )
                        },
                        Thread {
                            resolved: true,
                            comments: vec![sent(120, "drop this", Some(1))],
                            now: Some(ThreadPlace {
                                tree: state('e'),
                                path: PathBuf::from("src/lib.rs"),
                                side: Side::Old,
                                first: 3,
                                last: 3,
                                notice: Some(NoticeRecord::Deleted),
                            }),
                            ..thread(
                                12,
                                120,
                                "",
                                anchor(PathBuf::from("src/lib.rs"), Side::New, 3, 3),
                            )
                        },
                        Thread {
                            comments: vec![sent(130, "and this", Some(2))],
                            ..thread(
                                13,
                                130,
                                "",
                                anchor(PathBuf::from("src/lib.rs"), Side::New, 3, 3),
                            )
                        },
                    ],
                    rounds: vec![
                        Round {
                            number: 1,
                            state: Some(state('b')),
                            sent_by: SentBy::Delivered,
                            edited: true,
                        },
                        delivered(2, None),
                    ],
                    attached: Some(AttachedRecord::Thread(tid(11))),
                    ..review(1, Some("session-a"))
                },
                ReviewRecord {
                    threads: vec![thread(
                        20,
                        200,
                        "of no agent",
                        anchor(PathBuf::from("src/lib.rs"), Side::New, 3, 3),
                    )],
                    attached: Some(AttachedRecord::Review),
                    ..review(2, None)
                },
            ],
            turns: vec![
                TurnRecord {
                    ended: Some(TurnEndRecord::Finished),
                    start: Some(state('f')),
                    state: Some(state('9')),
                    ..turn(60, 1)
                },
                turn(61, 2),
            ],
            turn_with_steps: vec![
                (
                    TurnRecord {
                        ended: Some(TurnEndRecord::Finished),
                        start: Some(state('f')),
                        state: Some(state('9')),
                        ..turn(60, 1)
                    },
                    vec![
                        StepRecord::Read(odd.clone()),
                        StepRecord::Edit(odd.clone(), Some(vec![3..=4, 9..=9])),
                    ],
                ),
                (
                    turn(61, 2),
                    vec![StepRecord::Command("cargo test".to_owned())],
                ),
            ],
            session: Some(session),
        };
        assert_eq!(read_everything(&store), expected);
    }

    // Opened anew, twice, as after two restarts; the first Store is gone.
    for _ in 0..2 {
        let store = Store::open(&scratch.file()).expect("open again");
        assert_eq!(read_everything(&store), expected);
        for review in &expected.reviews {
            assert_eq!(
                store.review(&review.id).expect("review").as_ref(),
                Some(review)
            );
        }
    }
}

/// Round `number`, delivered with a message, with the state recorded for it or none yet.
fn delivered(number: u32, state: Option<String>) -> Round {
    Round {
        number,
        state,
        sent_by: SentBy::Delivered,
        edited: false,
    }
}

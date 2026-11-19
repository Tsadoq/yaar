//! Component tests: the calls of the Store that keep and find agent sessions, their turns
//! and the steps of each turn, on a real database file.

mod support;

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use support::Scratch;
use yaar_store::{
    EndedBy, ReviewId, ReviewRecord, SessionId, SessionRecord, StepRecord, Store, StoreError,
    TurnEndRecord, TurnId, TurnRecord, WorkspaceId,
};

/// An id of 32 hexadecimal digits, from a number.
fn id(n: u128) -> String {
    format!("{n:032x}")
}

fn workspace(n: u128) -> WorkspaceId {
    WorkspaceId::new(id(n))
}

fn sid(n: u128) -> SessionId {
    SessionId::new(id(n))
}

fn tid(n: u128) -> TurnId {
    TurnId::new(id(n))
}

/// A session `n` of workspace `workspace`, running in terminal `t<n>`, that its CLI calls
/// `cli-<n>`.
fn session(n: u128, workspace: u128) -> SessionRecord {
    SessionRecord {
        id: sid(n),
        workspace: self::workspace(workspace),
        agent: "claude".to_owned(),
        cli_session: format!("cli-{n}"),
        terminal: Some(format!("t{n}")),
        number: 1,
        running: true,
        ended: None,
        ended_by: None,
    }
}

/// A turn `n` of session `session`, still running.
fn turn(n: u128, session: u128, number: u32) -> TurnRecord {
    TurnRecord {
        id: tid(n),
        session: sid(session),
        number,
        ended: None,
        state: None,
    }
}

fn put_all(store: &Store, sessions: &[SessionRecord]) {
    for s in sessions {
        store.put_session(s).expect("put session");
    }
}

fn session_of(store: &Store, n: u128) -> SessionRecord {
    store
        .session(&sid(n))
        .expect("read the session")
        .expect("the session is kept")
}

fn refusal_of<T: std::fmt::Debug>(result: Result<T, StoreError>) -> String {
    match result {
        Err(StoreError::Database(sentence)) => sentence,
        other => panic!("not refused with a sentence: {other:?}"),
    }
}

fn ids(sessions: &[SessionRecord]) -> Vec<SessionId> {
    sessions.iter().map(|s| s.id.clone()).collect()
}

#[test]
fn every_call_keeps_what_it_is_given_and_a_store_opened_later_finds_it_again() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    // A session that ended before it was kept: its end and how it came are kept as given.
    let ended = SessionRecord {
        running: false,
        ended: Some(7),
        ended_by: Some(EndedBy::Cli),
        number: 2,
        ..session(2, 1)
    };
    let elsewhere = SessionRecord {
        agent: "pi".to_owned(),
        terminal: None,
        ..session(3, 2)
    };
    put_all(&store, &[session(1, 1), ended.clone(), elsewhere.clone()]);

    store.put_turn(&turn(10, 1, 1)).expect("put turn");
    store
        .end_turn(&tid(10), TurnEndRecord::Finished, Some(&"a".repeat(40)))
        .expect("end turn");
    store.put_turn(&turn(11, 1, 2)).expect("put turn");
    store
        .end_turn(&tid(11), TurnEndRecord::Interrupted, None)
        .expect("end turn");
    store.put_turn(&turn(12, 2, 1)).expect("put turn");
    let odd = PathBuf::from(OsStr::from_bytes(b"src/\xff odd\n'name\".rs"));
    let steps = [
        StepRecord::Read(odd.clone()),
        StepRecord::Edit(odd.clone(), Some(vec![3..=3, 10..=12])),
        StepRecord::Write(PathBuf::from("new.txt")),
        StepRecord::Command("echo \"a b\"\n  && ls".to_owned()),
    ];
    for (n, step) in steps.iter().enumerate() {
        store
            .put_step(&tid(10), n as u32 + 1, step)
            .expect("put step");
    }
    drop(store);

    let store = Store::open(&scratch.file()).expect("open again");
    assert_eq!(session_of(&store, 1), session(1, 1));
    assert_eq!(session_of(&store, 2), ended);
    assert_eq!(session_of(&store, 3), elsewhere);
    assert_eq!(
        store.session(&sid(99)).expect("read"),
        None,
        "a session that is not kept"
    );
    let first = store
        .turn(&tid(10))
        .expect("read the turn")
        .expect("the turn is kept");
    assert_eq!(
        first.0,
        TurnRecord {
            ended: Some(TurnEndRecord::Finished),
            state: Some("a".repeat(40)),
            ..turn(10, 1, 1)
        }
    );
    assert_eq!(first.1, steps);
    let second = store.turn(&tid(11)).expect("read").expect("kept");
    assert_eq!(
        second.0,
        TurnRecord {
            ended: Some(TurnEndRecord::Interrupted),
            state: None,
            ..turn(11, 1, 2)
        }
    );
    assert_eq!(second.1, []);
    let running = store.turn(&tid(12)).expect("read").expect("kept");
    assert_eq!((running.0, running.1), (turn(12, 2, 1), vec![]));
    assert_eq!(store.turn(&tid(99)).expect("read"), None);
}

#[test]
fn the_sessions_of_a_workspace_come_in_the_order_they_were_first_kept_and_no_others() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    assert_eq!(store.sessions_of(&workspace(1)).expect("none yet"), []);
    // Kept in an order that is neither the order of their ids nor of their numbers.
    put_all(
        &store,
        &[
            session(5, 1),
            session(2, 2),
            session(9, 1),
            session(1, 1),
            session(4, 2),
        ],
    );

    assert_eq!(
        ids(&store.sessions_of(&workspace(1)).expect("of 1")),
        [sid(5), sid(9), sid(1)]
    );
    assert_eq!(
        ids(&store.sessions_of(&workspace(2)).expect("of 2")),
        [sid(2), sid(4)]
    );
    assert_eq!(store.sessions_of(&workspace(3)).expect("of 3"), []);
}

#[test]
fn a_session_is_found_by_the_name_its_cli_gives_it_in_any_workspace() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    let pi = SessionRecord {
        agent: "pi".to_owned(),
        cli_session: "cli-1".to_owned(),
        ..session(2, 2)
    };
    put_all(&store, &[session(1, 1), pi.clone(), session(3, 2)]);

    let found = store
        .session_of_cli("claude", "cli-1")
        .expect("find")
        .expect("found");
    assert_eq!(found.id, sid(1));
    assert_eq!(found.workspace, workspace(1));
    // The same name under another CLI is another session.
    assert_eq!(store.session_of_cli("pi", "cli-1").expect("find"), Some(pi));
    assert_eq!(
        store
            .session_of_cli("claude", "cli-3")
            .expect("find")
            .map(|s| s.id),
        Some(sid(3))
    );
    assert_eq!(store.session_of_cli("claude", "cli-9").expect("find"), None);
    assert_eq!(store.session_of_cli("pi", "cli-3").expect("find"), None);
}

#[test]
fn a_session_id_or_cli_name_taken_is_refused_and_what_is_kept_stays() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    put_all(&store, &[session(1, 1)]);

    // The id is taken, in another workspace and with other fields.
    let sentence = refusal_of(store.put_session(&SessionRecord {
        cli_session: "other".to_owned(),
        ..session(1, 2)
    }));
    assert!(sentence.contains(&id(1)), "{sentence}");
    // The CLI's name for it is taken under the same Adapter.
    let sentence = refusal_of(store.put_session(&SessionRecord {
        cli_session: "cli-1".to_owned(),
        ..session(2, 2)
    }));
    assert!(sentence.contains("cli-1"), "{sentence}");

    assert_eq!(session_of(&store, 1), session(1, 1));
    assert_eq!(store.session(&sid(2)).expect("read"), None);
    assert_eq!(store.sessions_of(&workspace(2)).expect("of 2"), []);
}

#[test]
fn an_end_gets_one_more_than_the_largest_and_how_it_came_is_kept_each_of_three_ways() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    put_all(
        &store,
        &[session(1, 1), session(2, 1), session(3, 2), session(4, 2)],
    );

    store.set_ended(&sid(1), EndedBy::Cli).expect("end 1");
    store
        .set_ended(&sid(2), EndedBy::ProcessGone)
        .expect("end 2");
    store
        .set_ended(&sid(3), EndedBy::KeeperStopped)
        .expect("end 3");
    for (n, ended, by) in [
        (1, 1, EndedBy::Cli),
        (2, 2, EndedBy::ProcessGone),
        (3, 3, EndedBy::KeeperStopped),
    ] {
        let kept = session_of(&store, n);
        assert_eq!(
            (kept.running, kept.ended, kept.ended_by),
            (false, Some(ended), Some(by)),
            "session {n}"
        );
    }
    // The number is the largest of all the workspaces', and one that ends later has the
    // larger.
    store.set_ended(&sid(4), EndedBy::Cli).expect("end 4");
    assert_eq!(session_of(&store, 4).ended, Some(4));
    // A session that ends again, as one resumed and quit again, gets the next number and
    // the new way.
    store.set_ended(&sid(1), EndedBy::ProcessGone).expect("end");
    let again = session_of(&store, 1);
    assert_eq!(
        (again.ended, again.ended_by),
        (Some(5), Some(EndedBy::ProcessGone))
    );

    // Nothing else of a session changes with its end.
    let mut expected = session(2, 1);
    expected.running = false;
    expected.ended = Some(2);
    expected.ended_by = Some(EndedBy::ProcessGone);
    assert_eq!(session_of(&store, 2), expected);

    let sentence = refusal_of(store.set_ended(&sid(99), EndedBy::Cli));
    assert!(sentence.contains(&id(99)), "{sentence}");
}

#[test]
fn a_session_marked_running_has_neither_end_and_the_terminal_it_runs_in() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    put_all(&store, &[session(1, 1), session(2, 1)]);
    store
        .set_ended(&sid(1), EndedBy::KeeperStopped)
        .expect("end");
    store.set_ended(&sid(2), EndedBy::Cli).expect("end");

    store.set_running(&sid(1), Some("t9")).expect("resume");
    let resumed = session_of(&store, 1);
    assert_eq!(
        resumed,
        SessionRecord {
            terminal: Some("t9".to_owned()),
            ..session(1, 1)
        }
    );
    // The other one is as it was.
    let quit = session_of(&store, 2);
    assert_eq!(
        (quit.running, quit.ended, quit.ended_by),
        (false, Some(2), Some(EndedBy::Cli))
    );

    // Running in no terminal clears the terminal.
    store.set_running(&sid(1), None).expect("resume");
    assert_eq!(session_of(&store, 1).terminal, None);

    let sentence = refusal_of(store.set_running(&sid(99), None));
    assert!(sentence.contains(&id(99)), "{sentence}");
}

#[test]
fn only_the_sessions_that_run_are_listed_in_every_workspace() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    assert_eq!(store.running_sessions().expect("none yet"), []);
    put_all(
        &store,
        &[
            session(1, 1),
            session(2, 1),
            session(3, 2),
            session(4, 2),
            SessionRecord {
                running: false,
                ..session(5, 3)
            },
        ],
    );
    store.set_ended(&sid(2), EndedBy::Cli).expect("end");
    store.set_ended(&sid(3), EndedBy::ProcessGone).expect("end");
    assert_eq!(
        ids(&store.running_sessions().expect("running")),
        [sid(1), sid(4)]
    );

    // One that is resumed is listed again, in the place it was first kept.
    store.set_running(&sid(2), None).expect("resume");
    assert_eq!(
        ids(&store.running_sessions().expect("running")),
        [sid(1), sid(2), sid(4)]
    );
    store
        .set_ended(&sid(1), EndedBy::KeeperStopped)
        .expect("end");
    store
        .set_ended(&sid(2), EndedBy::KeeperStopped)
        .expect("end");
    store
        .set_ended(&sid(4), EndedBy::KeeperStopped)
        .expect("end");
    assert_eq!(store.running_sessions().expect("none left"), []);
}

fn review(n: u128, workspace: u128, agent: Option<&SessionId>) -> ReviewRecord {
    ReviewRecord {
        id: ReviewId::new(id(n)),
        workspace: self::workspace(workspace),
        agent: agent.map(|s| s.as_str().to_owned()),
        reviewed: None,
        threads: vec![],
        rounds: vec![],
    }
}

fn agent_of(store: &Store, review: u128) -> Option<String> {
    store
        .review(&ReviewId::new(id(review)))
        .expect("read the review")
        .expect("the review is kept")
        .agent
}

#[test]
fn a_review_of_no_agent_given_to_a_session_leaves_room_for_a_new_one_of_no_agent() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    put_all(&store, &[session(1, 1), session(2, 1)]);
    store.put_review(&review(10, 1, None)).expect("put review");
    assert!(
        store.put_review(&review(11, 1, None)).is_err(),
        "a second review of no agent is refused while the first is of none"
    );

    store
        .set_review_agent(&ReviewId::new(id(10)), Some(&sid(1)))
        .expect("give the review");
    assert_eq!(agent_of(&store, 10), Some(id(1)));
    // The unique index allows a new review of no agent now, and no second of that one.
    store
        .put_review(&review(11, 1, None))
        .expect("a new review of no agent");
    assert_eq!(agent_of(&store, 11), None);
    assert!(store.put_review(&review(12, 1, None)).is_err());

    // A review of one session goes to another, as a takeover does.
    store
        .set_review_agent(&ReviewId::new(id(10)), Some(&sid(2)))
        .expect("move the review");
    assert_eq!(agent_of(&store, 10), Some(id(2)));
    // Other workspaces keep their own review of no agent.
    store.put_review(&review(13, 2, None)).expect("workspace 2");
}

#[test]
fn a_review_given_back_to_no_agent_is_refused_while_the_workspace_has_one_already() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    put_all(&store, &[session(1, 1)]);
    store
        .put_review(&review(10, 1, Some(&sid(1))))
        .expect("put review");
    store.put_review(&review(11, 1, None)).expect("put review");

    let sentence = refusal_of(store.set_review_agent(&ReviewId::new(id(10)), None));
    assert!(
        sentence.contains("review of no agent already"),
        "{sentence}"
    );
    assert_eq!(agent_of(&store, 10), Some(id(1)));

    // With none of no agent in the way it is allowed.
    store
        .set_review_agent(&ReviewId::new(id(11)), Some(&sid(1)))
        .expect("give the review");
    store
        .set_review_agent(&ReviewId::new(id(10)), None)
        .expect("give back");
    assert_eq!(agent_of(&store, 10), None);
}

#[test]
fn a_review_or_a_session_the_store_does_not_hold_is_refused_when_a_review_is_given() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    put_all(&store, &[session(1, 1)]);
    store.put_review(&review(10, 1, None)).expect("put review");

    let sentence = refusal_of(store.set_review_agent(&ReviewId::new(id(99)), Some(&sid(1))));
    assert!(sentence.contains(&id(99)), "{sentence}");
    let sentence = refusal_of(store.set_review_agent(&ReviewId::new(id(10)), Some(&sid(99))));
    assert!(sentence.contains(&id(99)), "{sentence}");
    assert_eq!(agent_of(&store, 10), None);
}

#[test]
fn the_steps_of_a_turn_come_back_in_number_order_with_the_lines_of_each_edit_as_kept() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    put_all(&store, &[session(1, 1)]);
    store.put_turn(&turn(10, 1, 1)).expect("put turn");
    store.put_turn(&turn(11, 1, 2)).expect("put turn");

    let edit_many = StepRecord::Edit(PathBuf::from("a.rs"), Some(vec![1..=1, 20..=31]));
    let edit_none_said = StepRecord::Edit(PathBuf::from("b.rs"), None);
    let edit_empty = StepRecord::Edit(PathBuf::from("c.rs"), Some(vec![]));
    let command = StepRecord::Command("cargo test".to_owned());
    // Put out of order: a reader still gets them by number.
    store.put_step(&tid(10), 3, &edit_none_said).expect("put 3");
    store.put_step(&tid(10), 1, &edit_many).expect("put 1");
    store.put_step(&tid(10), 4, &edit_empty).expect("put 4");
    store.put_step(&tid(10), 2, &command).expect("put 2");
    // A step of another turn, with a number of its own.
    store
        .put_step(&tid(11), 1, &StepRecord::Read(PathBuf::from("a.rs")))
        .expect("put");

    let (_, steps) = store.turn(&tid(10)).expect("read").expect("kept");
    assert_eq!(steps, [edit_many, command, edit_none_said, edit_empty]);
    let (_, steps) = store.turn(&tid(11)).expect("read").expect("kept");
    assert_eq!(steps, [StepRecord::Read(PathBuf::from("a.rs"))]);
}

#[test]
fn a_step_number_taken_twice_is_refused_and_the_first_step_stays() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    put_all(&store, &[session(1, 1)]);
    store.put_turn(&turn(10, 1, 1)).expect("put turn");
    store
        .put_step(&tid(10), 1, &StepRecord::Read(PathBuf::from("a.rs")))
        .expect("put 1");

    let sentence =
        refusal_of(store.put_step(&tid(10), 1, &StepRecord::Write(PathBuf::from("b.rs"))));
    assert!(sentence.contains("step 1"), "{sentence}");
    let (_, steps) = store.turn(&tid(10)).expect("read").expect("kept");
    assert_eq!(steps, [StepRecord::Read(PathBuf::from("a.rs"))]);

    // Steps count from 1, and belong to a turn that is kept.
    let step = StepRecord::Command("ls".to_owned());
    assert!(store.put_step(&tid(10), 0, &step).is_err());
    let sentence = refusal_of(store.put_step(&tid(99), 1, &step));
    assert!(sentence.contains(&id(99)), "{sentence}");
    let (_, steps) = store.turn(&tid(10)).expect("read").expect("kept");
    assert_eq!(steps.len(), 1);
}

#[test]
fn a_turn_is_kept_once_for_a_session_that_is_kept_and_a_number_of_it() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    put_all(&store, &[session(1, 1)]);
    store.put_turn(&turn(10, 1, 1)).expect("put turn");

    // The id is taken; the number is taken in that session; the session is not kept.
    let sentence = refusal_of(store.put_turn(&turn(10, 1, 2)));
    assert!(sentence.contains(&id(10)), "{sentence}");
    let sentence = refusal_of(store.put_turn(&turn(11, 1, 1)));
    assert!(sentence.contains("turn 1"), "{sentence}");
    let sentence = refusal_of(store.put_turn(&turn(12, 99, 1)));
    assert!(sentence.contains(&id(99)), "{sentence}");
    assert_eq!(
        store.turns_of(&workspace(1)).expect("turns"),
        [turn(10, 1, 1)]
    );
}

#[test]
fn the_end_of_a_turn_keeps_how_it_ended_and_its_state_and_a_turn_not_kept_is_refused() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    put_all(&store, &[session(1, 1)]);
    for n in 1..=3 {
        store
            .put_turn(&turn(10 + u128::from(n), 1, n))
            .expect("put turn");
    }
    let state = "b".repeat(40);
    store
        .end_turn(&tid(11), TurnEndRecord::Finished, Some(&state))
        .expect("end 1");
    store
        .end_turn(&tid(12), TurnEndRecord::Interrupted, Some(&state))
        .expect("end 2");

    let turns = store.turns_of(&workspace(1)).expect("turns");
    assert_eq!(
        turns
            .iter()
            .map(|t| (t.number, t.ended, t.state.as_deref()))
            .collect::<Vec<_>>(),
        [
            (1, Some(TurnEndRecord::Finished), Some(state.as_str())),
            (2, Some(TurnEndRecord::Interrupted), Some(state.as_str())),
            (3, None, None),
        ]
    );
    let sentence = refusal_of(store.end_turn(&tid(99), TurnEndRecord::Finished, None));
    assert!(sentence.contains(&id(99)), "{sentence}");
}

#[test]
fn the_turns_of_a_workspace_come_in_the_order_they_started_across_its_sessions() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    put_all(&store, &[session(1, 1), session(2, 1), session(3, 2)]);
    assert_eq!(store.turns_of(&workspace(1)).expect("none yet"), []);
    // Started in an order that is neither the order of the sessions nor of the ids.
    store.put_turn(&turn(30, 2, 1)).expect("put");
    store.put_turn(&turn(20, 1, 1)).expect("put");
    store.put_turn(&turn(40, 3, 1)).expect("put");
    store.put_turn(&turn(10, 2, 2)).expect("put");

    assert_eq!(
        store.turns_of(&workspace(1)).expect("of 1"),
        [turn(30, 2, 1), turn(20, 1, 1), turn(10, 2, 2)]
    );
    assert_eq!(
        store.turns_of(&workspace(2)).expect("of 2"),
        [turn(40, 3, 1)]
    );
    assert_eq!(store.turns_of(&workspace(3)).expect("of 3"), []);
}

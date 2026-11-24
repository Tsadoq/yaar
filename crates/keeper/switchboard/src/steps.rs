//! Tests of the steps of a turn, through [`crate::Switchboard::take`] and
//! [`crate::Switchboard::session`].

use std::path::PathBuf;

use yaar_agent::{CliSession, Event, Phase, Reported, Start, TurnEnd, WriteTool};

use super::{Change, Known, SessionId, Switchboard, TurnId};

const AGENT: &str = "claude";

struct Desk {
    board: Switchboard,
    draws: u32,
}

impl Desk {
    fn new() -> Desk {
        Desk {
            board: Switchboard::new(),
            draws: 0,
        }
    }

    fn hear(&mut self, cli: &str, pid: u32, event: Event) -> Vec<Change> {
        let draws = &mut self.draws;
        let reported = Reported {
            session: CliSession(cli.to_owned()),
            terminal: Some(format!("terminal-{cli}")),
            pid,
            folder: Some(PathBuf::from("/work/project")),
            cli_record: None,
            event,
        };
        self.board.take(AGENT, reported, &mut || {
            *draws += 1;
            format!("{draws:032x}")
        })
    }

    fn known(&self, cli: &str) -> Known {
        self.board
            .sessions()
            .into_iter()
            .find(|known| known.cli == CliSession(cli.to_owned()))
            .unwrap_or_else(|| panic!("no session {cli}"))
    }

    fn in_a_turn(&mut self, cli: &str, pid: u32) -> (SessionId, TurnId) {
        self.hear(cli, pid, Event::SessionStarted { how: Start::New });
        self.hear(cli, pid, Event::TurnStarted);
        let (turn, steps) = self.known(cli).turn.expect("a turn is open");
        assert_eq!(steps, 0, "no step has been told yet");
        (self.known(cli).id, turn)
    }
}

fn read(path: &str) -> Event {
    Event::FileRead {
        path: PathBuf::from(path),
    }
}

fn wrote(path: &str, tool: WriteTool, lines: Option<Vec<std::ops::RangeInclusive<u32>>>) -> Event {
    Event::FileWritten {
        path: PathBuf::from(path),
        tool,
        lines,
    }
}

fn make(phase: Phase) -> Event {
    Event::Command {
        phase,
        command: "make".to_owned(),
    }
}

fn step(session: &SessionId, turn: &TurnId, number: u32, event: Event) -> Change {
    Change::Step {
        session: session.clone(),
        turn: turn.clone(),
        number,
        event,
    }
}

#[test]
fn the_steps_of_a_turn_are_kept_in_order_each_with_its_session_its_kind_and_an_edit_with_its_path_and_lines()
 {
    let mut desk = Desk::new();
    let (a, a_turn) = desk.in_a_turn("a", 10);
    let (b, b_turn) = desk.in_a_turn("b", 20);
    assert_ne!(a, b);
    assert_ne!(a_turn, b_turn);

    // The two sessions' steps arrive mixed, as the hooks of two terminals do.
    let edit_lines = vec![3..=4, 9..=9];
    let fed = [
        ("a", 10, read("a.txt")),
        ("b", 20, wrote("b.txt", WriteTool::Write, None)),
        (
            "a",
            10,
            wrote("a.txt", WriteTool::Edit, Some(edit_lines.clone())),
        ),
        ("a", 10, make(Phase::Began)),
        ("a", 10, make(Phase::Ended)),
        ("b", 20, read("c.txt")),
        ("a", 10, wrote("new.txt", WriteTool::Write, None)),
    ];
    let told: Vec<Change> = fed
        .iter()
        .flat_map(|(cli, pid, event)| desk.hear(cli, *pid, event.clone()))
        .collect();

    // The end of the command is no step.
    let expected = [
        step(&a, &a_turn, 1, read("a.txt")),
        step(&b, &b_turn, 1, wrote("b.txt", WriteTool::Write, None)),
        step(
            &a,
            &a_turn,
            2,
            wrote("a.txt", WriteTool::Edit, Some(edit_lines)),
        ),
        step(&a, &a_turn, 3, make(Phase::Began)),
        Change::CommandEnded { session: a.clone() },
        step(&b, &b_turn, 2, read("c.txt")),
        step(&a, &a_turn, 4, wrote("new.txt", WriteTool::Write, None)),
    ];
    assert_eq!(told, expected);

    assert_eq!(desk.board.session(&a).expect("a").turn, Some((a_turn, 4)));
    assert_eq!(desk.board.session(&b).expect("b").turn, Some((b_turn, 2)));
}

#[test]
fn the_steps_of_the_next_turn_number_from_1_again_and_a_step_between_two_turns_is_none() {
    let mut desk = Desk::new();
    let (a, first) = desk.in_a_turn("a", 10);
    desk.hear("a", 10, read("a.txt"));
    desk.hear("a", 10, read("b.txt"));
    desk.hear(
        "a",
        10,
        Event::TurnEnded {
            how: TurnEnd::Finished,
        },
    );

    assert_eq!(desk.hear("a", 10, read("between.txt")), []);
    assert_eq!(desk.known("a").turn, None);

    desk.hear("a", 10, Event::TurnStarted);
    let (second, steps) = desk.known("a").turn.expect("the new turn is open");
    assert_ne!(second, first);
    assert_eq!(steps, 0);
    assert_eq!(
        desk.hear("a", 10, read("c.txt")),
        [step(&a, &second, 1, read("c.txt"))]
    );
}

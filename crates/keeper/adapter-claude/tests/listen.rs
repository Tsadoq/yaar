//! What a recorded Claude Code turn gives the app: the events of the agent's steps, in
//! order, each with its session, and the two reads the Adapter cannot see.
//!
//! The recorded hooks go in through the Adapter's call, as the Keeper hands them over, and
//! the events come out.

mod support;

use std::ops::RangeInclusive;

use yaar_adapter_claude::ClaudeCode;
use yaar_agent::{CliSession, Event, Phase, Reported, Start, TurnEnd, WriteTool};
use yaar_system_tests::recorded::{self, HookRecord};

use support::{command_of, feed, in_project, told};

/// The recording of one turn with a read, an edit, a new file, two shell commands, a
/// sub-agent and a turn Claude Code started by itself.
const TURN: &str = "claude/turn-tools.jsonl";

/// A turn whose prompt names a file with an @ mention.
const AT_MENTION: &str = "claude/at-mention.jsonl";

/// A turn that reads a file through a shell command, typed by hand: no recording holds it.
const SHELL_READ: &str = "claude-shell-read.jsonl";

/// The lines of [`TURN`] the row replays: the turn up to its last shell command, then its
/// end and the end of the session. The sub-agent's lines (21 to 31) and the turn the CLI
/// started itself (33 and 34) are left out.
fn lines_of_the_turn() -> Vec<usize> {
    (0..=20).chain([32, 35]).collect()
}

/// The events of the `lines` of `records`, fed in that order to one Adapter, each with the
/// number of the line that gave it.
fn replay_lines(records: &[HookRecord], lines: &[usize]) -> Vec<(usize, Reported)> {
    let mut adapter = ClaudeCode::new();
    let mut heard = Vec::new();
    for &line in lines {
        for reported in feed(&mut adapter, &records[line]) {
            heard.push((line, reported));
        }
    }
    heard
}

/// Every record of `records`, in order, fed to one Adapter.
fn replay_all(records: &[HookRecord]) -> Vec<(usize, Reported)> {
    let lines: Vec<usize> = (0..records.len()).collect();
    replay_lines(records, &lines)
}

/// What `heard` should be: the events, each told as its line's record gives it.
fn expected(records: &[HookRecord], events: Vec<(usize, Event)>) -> Vec<(usize, Reported)> {
    events
        .into_iter()
        .map(|(line, event)| (line, told(&records[line], event)))
        .collect()
}

fn began(command: String) -> Event {
    Event::Command {
        phase: Phase::Began,
        command,
    }
}

fn ended(command: String) -> Event {
    Event::Command {
        phase: Phase::Ended,
        command,
    }
}

/// The name of the kind of an event, with the two phases of a command one kind.
fn kind(event: &Event) -> &'static str {
    match event {
        Event::SessionStarted { .. } => "session started",
        Event::TurnStarted => "turn started",
        Event::FileRead { .. } => "file read",
        Event::FileWritten { .. } => "file written",
        Event::Command { .. } => "shell command",
        Event::Permission { .. } => "permission",
        Event::TurnEnded { .. } => "turn ended",
        Event::SessionEnded => "session ended",
    }
}

fn kinds_in_order_of_first_appearance(heard: &[(usize, Reported)]) -> Vec<&'static str> {
    let mut kinds = Vec::new();
    for (_, reported) in heard {
        let kind = kind(&reported.event);
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    kinds
}

#[test]
fn a_recorded_turn_with_a_read_an_edit_a_new_file_and_a_shell_write_gives_its_events_in_order() {
    let records = recorded::hook_events(TURN);
    let heard = replay_lines(&records, &lines_of_the_turn());

    // Every line gives what the recording shows it did, and the others give nothing: the
    // events are exactly these, in this order.
    let events = vec![
        (1, Event::SessionStarted { how: Start::New }),
        (2, Event::TurnStarted),
        (
            4,
            Event::FileRead {
                path: in_project("notes.txt"),
            },
        ),
        (
            7,
            Event::FileRead {
                path: in_project("src/calc.py"),
            },
        ),
        (
            10,
            Event::FileWritten {
                path: in_project("src/calc.py"),
                tool: WriteTool::Edit,
                lines: Some(vec![RangeInclusive::new(24, 28)]),
            },
        ),
        (
            13,
            Event::FileWritten {
                path: in_project("new.txt"),
                tool: WriteTool::Write,
                lines: None,
            },
        ),
        (15, began(command_of(&records[15]))),
        (16, ended(command_of(&records[16]))),
        (18, began(command_of(&records[18]))),
        (19, ended(command_of(&records[19]))),
        (
            32,
            Event::TurnEnded {
                how: TurnEnd::Finished,
            },
        ),
        (35, Event::SessionEnded),
    ];
    assert_eq!(heard, expected(&records, events));

    // The seven kinds, as they first appear. The seventh the Adapter knows, waiting for
    // permission, this turn never asks: a permission is shown by the dialogs' recording.
    assert_eq!(
        kinds_in_order_of_first_appearance(&heard),
        [
            "session started",
            "turn started",
            "file read",
            "file written",
            "shell command",
            "turn ended",
            "session ended"
        ]
    );
}

#[test]
fn every_event_of_the_turn_names_the_session_of_the_first_line_its_terminal_its_process_and_folder()
{
    let records = recorded::hook_events(TURN);
    let heard = replay_lines(&records, &lines_of_the_turn());

    let session = CliSession(records[1].session().to_owned());
    assert!(!heard.is_empty());
    for (line, reported) in &heard {
        assert_eq!(reported.session, session, "line {line}");
        assert_eq!(
            reported.terminal.as_deref(),
            records[*line].terminal(),
            "line {line}"
        );
        assert_eq!(reported.pid, records[*line].parent, "line {line}");
        assert_eq!(
            reported.folder.as_deref(),
            Some(std::path::Path::new(recorded::PROJECT)),
            "line {line}"
        );
    }
    // The recording holds one terminal, so that "the record's terminal" is a real name.
    assert!(records[1].terminal().is_some());
}

#[test]
fn a_file_named_by_an_at_mention_gives_no_read() {
    let records = recorded::hook_events(AT_MENTION);

    // The premise: the prompt names a file with an @ mention.
    let prompt = records[2].input["prompt"].as_str().expect("a prompt");
    assert!(prompt.contains("@new.txt"), "the prompt is {prompt:?}");

    let heard = replay_all(&records);
    let events = vec![
        (0, Event::SessionStarted { how: Start::New }),
        (2, Event::TurnStarted),
        (
            3,
            Event::TurnEnded {
                how: TurnEnd::Finished,
            },
        ),
        (4, Event::SessionEnded),
    ];
    assert_eq!(heard, expected(&records, events));
    assert!(
        heard
            .iter()
            .all(|(_, reported)| !matches!(reported.event, Event::FileRead { .. })),
        "the Adapter hears no tool call of the agent, so no read"
    );
}

#[test]
fn a_file_read_through_a_shell_command_gives_the_command_and_no_read() {
    // The premise: the loader gives the record of a shell command that only reads a file,
    // as its third record.
    let records = recorded::hand_made_hook_events(SHELL_READ);
    assert_eq!(records[2].event, "PreToolUse");
    assert_eq!(records[2].input["tool_name"], "Bash");
    assert_eq!(command_of(&records[2]), "cat notes.txt");

    let heard = replay_all(&records);
    let events = vec![
        (0, Event::SessionStarted { how: Start::New }),
        (1, Event::TurnStarted),
        (2, began("cat notes.txt".to_owned())),
        (3, ended("cat notes.txt".to_owned())),
        (
            4,
            Event::TurnEnded {
                how: TurnEnd::Finished,
            },
        ),
        (5, Event::SessionEnded),
    ];
    assert_eq!(heard, expected(&records, events));
    assert!(
        heard
            .iter()
            .all(|(_, reported)| !matches!(reported.event, Event::FileRead { .. })),
        "a shell command names no file to the Adapter"
    );
}

#[test]
fn a_shell_command_that_rewrites_a_file_the_agent_read_gives_commands_and_no_second_read() {
    let records = recorded::hook_events(TURN);
    let heard = replay_lines(&records, &lines_of_the_turn());

    // The premise: line 18 is a `sed -i` on `notes.txt`, a file line 4 read with the
    // agent's own tool.
    let command = command_of(&records[18]);
    assert!(
        command.contains("sed -i") && command.contains("notes.txt"),
        "line 18 is {command:?}"
    );

    let from_the_command: Vec<_> = heard
        .iter()
        .filter(|(line, _)| (18..=19).contains(line))
        .map(|(_, reported)| &reported.event)
        .collect();
    assert_eq!(from_the_command, [&began(command.clone()), &ended(command)]);
    let reads: Vec<usize> = heard
        .iter()
        .filter(|(_, reported)| matches!(reported.event, Event::FileRead { .. }))
        .map(|(line, _)| *line)
        .collect();
    assert_eq!(reads, [4, 7], "only the two reads of the agent's own tool");
}

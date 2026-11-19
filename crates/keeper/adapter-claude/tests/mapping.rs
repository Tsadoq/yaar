//! The mapping of Claude Code's hooks to the app's events, read off the recorded sessions
//! with tools, a sub-agent, a resume and an interrupt.

mod support;

use std::ops::RangeInclusive;

use yaar_adapter_claude::{ClaudeCode, INTERRUPT_MARKS};
use yaar_agent::{Adapter, Event, Phase, Start, TurnEnd, WriteTool};

use support::{command_of, feed, in_project, replay, told};
use yaar_system_tests::recorded::{self, HookRecord};

const TOOLS: &str = "claude/turn-tools.jsonl";
const RESUME: &str = "claude/interrupt-resume.jsonl";

/// The Adapter after the lines `0..=last` of the recording `name`, and its records.
fn replay_to(name: &str, last: usize) -> (ClaudeCode, Vec<HookRecord>) {
    let records = recorded::hook_events(name);
    let mut adapter = ClaudeCode::new();
    for record in &records[..=last] {
        feed(&mut adapter, record);
    }
    (adapter, records)
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

#[test]
fn a_turn_with_reads_edits_writes_and_shell_commands_gives_each_its_event() {
    let (_, records, by_line) = replay(TOOLS);
    let expect = |line: usize, events: Vec<Event>| {
        let told: Vec<_> = events
            .into_iter()
            .map(|event| told(&records[line], event))
            .collect();
        assert_eq!(by_line[line], told, "line {line}");
    };

    expect(1, vec![Event::SessionStarted { how: Start::New }]);
    expect(2, vec![Event::TurnStarted]);
    expect(
        4,
        vec![Event::FileRead {
            path: in_project("notes.txt"),
        }],
    );
    expect(
        7,
        vec![Event::FileRead {
            path: in_project("src/calc.py"),
        }],
    );
    expect(
        10,
        vec![Event::FileWritten {
            path: in_project("src/calc.py"),
            tool: WriteTool::Edit,
            lines: Some(vec![RangeInclusive::new(24, 28)]),
        }],
    );
    expect(
        13,
        vec![Event::FileWritten {
            path: in_project("new.txt"),
            tool: WriteTool::Write,
            lines: None,
        }],
    );
    expect(15, vec![began(command_of(&records[15]))]);
    expect(16, vec![ended(command_of(&records[16]))]);
    expect(18, vec![began(command_of(&records[18]))]);
    expect(19, vec![ended(command_of(&records[19]))]);
    expect(
        32,
        vec![Event::TurnEnded {
            how: TurnEnd::Finished,
        }],
    );
    expect(35, vec![Event::SessionEnded]);
}

#[test]
fn hooks_that_name_no_step_of_the_agent_give_nothing() {
    let (_, _, by_line) = replay(TOOLS);

    // Instructions loaded, the pre-hook of a read, the end of a batch, the pre-hooks of an
    // edit and a write, a sub-agent, the end of one.
    for line in [0, 3, 5, 6, 8, 9, 11, 12, 14, 17, 20, 21, 22, 23, 24, 31] {
        assert_eq!(by_line[line], [], "line {line}");
    }
}

#[test]
fn every_event_names_the_project_folder_its_terminal_and_one_session() {
    let (_, records, by_line) = replay(TOOLS);

    let heard: Vec<_> = by_line.iter().flatten().collect();
    assert!(heard.len() >= 12, "the session says something of each step");
    for reported in heard {
        assert_eq!(
            reported.folder.as_deref(),
            Some(std::path::Path::new(recorded::PROJECT))
        );
        assert_eq!(reported.terminal.as_deref(), records[0].terminal());
        assert_eq!(reported.session.0, records[0].session());
    }
}

#[test]
fn what_a_sub_agent_does_is_the_sessions_own() {
    let (_, records, by_line) = replay(TOOLS);

    assert_eq!(
        by_line[26],
        [told(
            &records[26],
            Event::FileRead {
                path: in_project("README.md")
            }
        )]
    );
    assert_eq!(
        by_line[29],
        [told(
            &records[29],
            Event::FileWritten {
                path: in_project("README.md"),
                tool: WriteTool::Edit,
                lines: Some(vec![RangeInclusive::new(1, 4)]),
            }
        )]
    );
}

#[test]
fn the_turn_claude_code_starts_itself_for_a_finished_task_is_a_turn() {
    let (_, records, by_line) = replay(TOOLS);

    assert_eq!(by_line[33], [told(&records[33], Event::TurnStarted)]);
    assert_eq!(
        by_line[34],
        [told(
            &records[34],
            Event::TurnEnded {
                how: TurnEnd::Finished
            }
        )]
    );
}

#[test]
fn a_file_named_with_an_at_mention_gives_no_read() {
    let (_, _, by_line) = replay("claude/at-mention.jsonl");

    let events: Vec<_> = by_line.into_iter().flatten().map(|r| r.event).collect();
    assert_eq!(
        events,
        [
            Event::SessionStarted { how: Start::New },
            Event::TurnStarted,
            Event::TurnEnded {
                how: TurnEnd::Finished
            },
            Event::SessionEnded,
        ]
    );
}

#[test]
fn a_resumed_session_starts_resumed_under_the_name_it_had() {
    let (_, records, by_line) = replay(RESUME);

    assert_eq!(
        by_line[0],
        [told(&records[0], Event::SessionStarted { how: Start::New })]
    );
    for line in [8, 11] {
        assert_eq!(
            by_line[line],
            [told(
                &records[line],
                Event::SessionStarted {
                    how: Start::Resumed
                }
            )],
            "line {line}"
        );
        assert_eq!(records[line].session(), records[0].session());
    }
    assert_ne!(
        records[14].session(),
        records[0].session(),
        "a session started anew has a name of its own"
    );
}

#[test]
fn a_new_prompt_while_a_turn_is_open_is_told_as_it_comes() {
    // Line 3 is a shell command that Esc interrupted, line 4 the next message typed: the
    // CLI ran no hook between them, and the Switchboard is the one that ends the turn.
    let (_, records, by_line) = replay(RESUME);

    assert_eq!(
        by_line[3],
        [told(
            &records[3],
            Event::Command {
                phase: Phase::Began,
                command: command_of(&records[3]),
            }
        )]
    );
    assert_eq!(by_line[4], [told(&records[4], Event::TurnStarted)]);
}

#[test]
fn the_mark_after_an_interrupted_shell_command_ends_the_turn_without_a_permission() {
    // After line 3 a turn is open and a command begun, and Esc ran no hook.
    let (mut adapter, records) = replay_to(RESUME, 3);
    let terminal = records[3].terminal().expect("a terminal");

    assert_eq!(
        adapter.marked(terminal, 0),
        [told(
            &records[3],
            Event::TurnEnded {
                how: TurnEnd::Interrupted
            }
        )]
    );
}

#[test]
fn the_mark_of_a_session_that_has_ended_means_nothing() {
    // Line 7 is the end of the session of terminal A.
    let (mut adapter, records) = replay_to(RESUME, 7);
    let terminal = records[3].terminal().expect("a terminal");

    assert_eq!(adapter.marked(terminal, 0), []);
}

#[test]
fn every_form_of_the_mark_means_the_same_and_no_other_index_means_anything() {
    let marks = ClaudeCode::new().marks();
    assert_eq!(marks, INTERRUPT_MARKS.map(|mark| mark.to_vec()));

    for mark in 0..marks.len() {
        let (mut adapter, records) = replay_to(RESUME, 3);
        let terminal = records[3].terminal().expect("a terminal");
        assert_eq!(adapter.marked(terminal, mark).len(), 1, "mark {mark}");
    }
    let (mut adapter, records) = replay_to(RESUME, 3);
    let terminal = records[3].terminal().expect("a terminal");
    assert_eq!(adapter.marked(terminal, marks.len()), [], "no such mark");
}

#[test]
fn a_terminal_that_no_hook_came_from_has_no_mark_to_tell() {
    let (mut adapter, _) = replay_to(RESUME, 3);

    assert_eq!(adapter.marked("term-nowhere", 0), []);
}

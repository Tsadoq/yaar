//! A permission dialog in Claude Code turns waiting for permission on, and its answer turns
//! it off: the recorded hooks and the recorded mark go in through the Adapter's calls, and
//! the events come out.

mod support;

use yaar_adapter_claude::{ClaudeCode, INTERRUPT_MARKS};
use yaar_agent::{Adapter, Event, Phase, Reported, TurnEnd, WriteTool};
use yaar_system_tests::recorded::{self, HookRecord};

use support::{command_of, feed, in_project, told};

/// The recording of the dialogs: a Write accepted at lines 3 to 6, a Bash denied at lines
/// 11 and 12.
const DIALOGS: &str = "claude/permission-dialogs.jsonl";

/// The recording of the mark Claude Code writes when a dialog is denied or a turn
/// interrupted.
const MARK: &str = "claude/interrupt-mark.jsonl";

/// The Adapter after the lines `0..=last` of the dialogs' recording, the events of the
/// last one, and the record of it.
fn replay_to(last: usize) -> (ClaudeCode, Vec<Reported>, HookRecord) {
    let records = recorded::hook_events(DIALOGS);
    let mut adapter = ClaudeCode::new();
    let mut events = Vec::new();
    for record in &records[..=last] {
        events = feed(&mut adapter, record);
    }
    (adapter, events, records[last].clone())
}

/// The index, in the Adapter's marks, of the first mark the recorded chunk 0 holds.
fn recorded_mark() -> usize {
    let chunk = &recorded::terminal_chunks(MARK)[0];
    INTERRUPT_MARKS
        .iter()
        .position(|mark| chunk.windows(mark.len()).any(|window| window == *mark))
        .expect("the recorded chunk holds one of the marks the Adapter watches for")
}

#[test]
fn a_permission_dialog_turns_waiting_for_permission_on() {
    let (_, events, record) = replay_to(4);

    assert_eq!(
        events,
        [told(&record, Event::Permission { waiting: true })],
        "line 4 is the PermissionRequest of the dialog"
    );
}

#[test]
fn the_notification_that_follows_the_dialog_by_seconds_says_nothing() {
    let (_, events, record) = replay_to(5);

    assert_eq!(record.event, "Notification", "line 5 is the Notification");
    assert_eq!(events, [], "it comes 6 s after the dialog");
}

#[test]
fn the_answer_yes_turns_waiting_off_and_then_tells_the_write() {
    let (_, events, record) = replay_to(6);

    assert_eq!(
        events,
        [
            told(&record, Event::Permission { waiting: false }),
            told(
                &record,
                Event::FileWritten {
                    path: in_project("p.txt"),
                    tool: WriteTool::Write,
                    lines: None,
                }
            ),
        ],
        "line 6 is the PostToolUse of the write the dialog asked about"
    );
}

#[test]
fn the_answer_no_turns_waiting_off_by_the_mark_and_ends_the_turn() {
    let (mut adapter, events, record) = replay_to(12);
    assert_eq!(
        events,
        [told(&record, Event::Permission { waiting: true })],
        "line 12 is the PermissionRequest of the shell command"
    );

    let mark = recorded_mark();
    assert_eq!(adapter.marks()[mark], INTERRUPT_MARKS[mark]);
    let terminal = record
        .terminal()
        .expect("the recorded hook ran in a terminal");
    assert_eq!(
        adapter.marked(terminal, mark),
        [
            told(&record, Event::Permission { waiting: false }),
            told(
                &record,
                Event::TurnEnded {
                    how: TurnEnd::Interrupted
                }
            ),
        ],
        "a denied dialog runs no hook: the mark is all there is"
    );
}

#[test]
fn the_lines_before_the_denied_dialog_start_the_turn_and_begin_the_command() {
    let records = recorded::hook_events(DIALOGS);
    let mut adapter = ClaudeCode::new();
    let mut by_line = Vec::new();
    for record in &records[..=12] {
        by_line.push(feed(&mut adapter, record));
    }

    assert_eq!(by_line[10], [told(&records[10], Event::TurnStarted)]);
    assert_eq!(
        by_line[11],
        [told(
            &records[11],
            Event::Command {
                phase: Phase::Began,
                command: command_of(&records[11])
            }
        )]
    );
}

#[test]
fn the_mark_after_a_finished_turn_means_nothing() {
    let (mut adapter, _, record) = replay_to(8);

    let terminal = record
        .terminal()
        .expect("the recorded hook ran in a terminal");
    assert_eq!(
        adapter.marked(terminal, recorded_mark()),
        [],
        "line 8 is the Stop: the mark that follows it is the turn's own end"
    );
}

#[test]
fn the_mark_ends_the_turn_once() {
    let (mut adapter, _, record) = replay_to(12);
    let terminal = record
        .terminal()
        .expect("the recorded hook ran in a terminal");
    let mark = recorded_mark();
    assert_eq!(adapter.marked(terminal, mark).len(), 2);

    assert_eq!(adapter.marked(terminal, mark), []);
}

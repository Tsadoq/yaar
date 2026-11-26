mod support;

use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use yaar_adapter_pi::{Pi, REPLY_EVENT};
use yaar_agent::{
    Adapter, CliSession, Event, Phase, Report, ReportError, Reported, Start, TurnEnd, WriteTool,
};
use yaar_system_tests::recorded::{self, PiRecord};

use support::{Scratch, feed, told};

/// pi's four tools, one prompt each, in five runs, then a new session, a reload, a resume and a
/// quit.
const TURNS: &str = "pi/turn-tools.jsonl";

const AT_MENTION: &str = "pi-at-mention.jsonl";

const SHELL_READ: &str = "pi-shell-read.jsonl";

const RECORDINGS: [&str; 5] = [
    "pi/turn-tools.jsonl",
    "pi/queued-message.jsonl",
    "pi/resume-quit-abort.jsonl",
    "pi/reply-tool.jsonl",
    "pi/review-before-agent-start.jsonl",
];

fn replay(records: &[PiRecord]) -> Vec<(usize, Reported)> {
    let mut adapter = Pi::new();
    let mut heard = Vec::new();
    for (line, record) in records.iter().enumerate() {
        for reported in feed(&mut adapter, record) {
            heard.push((line, reported));
        }
    }
    heard
}

fn expected(records: &[PiRecord], events: Vec<(usize, Event)>) -> Vec<(usize, Reported)> {
    events
        .into_iter()
        .map(|(line, event)| (line, told(&records[line], event)))
        .collect()
}

fn began(command: &str) -> Event {
    Event::Command {
        phase: Phase::Began,
        command: command.to_owned(),
    }
}

fn ended(command: &str) -> Event {
    Event::Command {
        phase: Phase::Ended,
        command: command.to_owned(),
    }
}

fn finished() -> Event {
    Event::TurnEnded {
        how: TurnEnd::Finished,
    }
}

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
fn recorded_pi_runs_with_a_read_an_edit_a_new_file_and_a_shell_write_give_the_same_seven_events_in_order()
 {
    let scratch = Scratch::new();
    let project = scratch.project();
    let records: Vec<PiRecord> = recorded::pi_events(TURNS)
        .iter()
        .map(|record| record.in_project(&project))
        .collect();

    // Four tools in five runs, one prompt each. Line 87's command writes `bash_made.txt`; line
    // 40 is the edit tool's result, with the patch the lines come from.
    for (line, tool) in [(16, "read"), (40, "edit"), (64, "write"), (88, "bash")] {
        assert_eq!(records[line].event, "tool_result", "line {line}");
        assert_eq!(records[line].payload["toolName"], tool, "line {line}");
    }
    let shell_write = records[87].payload["input"]["command"]
        .as_str()
        .expect("the command");
    assert!(
        shell_write.contains('>') && shell_write.contains("bash_made.txt"),
        "line 87 is {shell_write:?}"
    );
    let patch = records[40].payload["details"]["patch"]
        .as_str()
        .expect("the edit's patch");
    assert!(patch.contains("@@ -1,9 +1,9 @@"), "the patch is {patch:?}");

    let heard = replay(&records);

    let events = vec![
        (1, Event::SessionStarted { how: Start::New }),
        (3, Event::TurnStarted),
        (
            16,
            Event::FileRead {
                path: project.join("src/calc.py"),
                lines: None,
            },
        ),
        (28, finished()),
        (29, Event::TurnStarted),
        (
            40,
            Event::FileWritten {
                path: project.join("src/calc.py"),
                tool: WriteTool::Edit,
                lines: Some(vec![RangeInclusive::new(1, 9)]),
            },
        ),
        (52, finished()),
        (53, Event::TurnStarted),
        (
            64,
            Event::FileWritten {
                path: project.join("new_file.txt"),
                tool: WriteTool::Write,
                lines: None,
            },
        ),
        (76, finished()),
        (77, Event::TurnStarted),
        (87, began("echo x > bash_made.txt")),
        (88, ended("echo x > bash_made.txt")),
        (100, finished()),
        (101, Event::TurnStarted),
        (111, began("sed -i '' 's/notes/NOTES/' notes.txt")),
        (112, ended("sed -i '' 's/notes/NOTES/' notes.txt")),
        (124, finished()),
        (127, Event::SessionEnded),
        (129, Event::SessionStarted { how: Start::New }),
        (131, Event::TurnStarted),
        (145, finished()),
        // The reload at 146 and 148 gives nothing; the resume ends the session at 151 and brings it
        // back at 153.
        (151, Event::SessionEnded),
        (
            153,
            Event::SessionStarted {
                how: Start::Resumed,
            },
        ),
        (155, Event::SessionEnded),
    ];
    assert_eq!(heard, expected(&records, events));

    let mut kinds = kinds_in_order_of_first_appearance(&heard);
    kinds.sort_unstable();
    assert_eq!(
        kinds,
        [
            "file read",
            "file written",
            "session ended",
            "session started",
            "shell command",
            "turn ended",
            "turn started"
        ]
    );
    let mut runs: Vec<Vec<&str>> = Vec::new();
    for (_, reported) in heard.iter().filter(|(line, _)| (3..=124).contains(line)) {
        let kind = kind(&reported.event);
        if kind == "turn started" {
            runs.push(Vec::new());
        }
        runs.last_mut().expect("a run opens first").push(kind);
    }
    let run = |step: &'static str, steps: usize| {
        let mut run = vec!["turn started"];
        run.extend(std::iter::repeat_n(step, steps));
        run.push("turn ended");
        run
    };
    assert_eq!(
        runs,
        [
            run("file read", 1),
            run("file written", 1),
            run("file written", 1),
            run("shell command", 2),
            run("shell command", 2),
        ]
    );
    let writes: Vec<&Path> = heard
        .iter()
        .filter_map(|(_, reported)| match &reported.event {
            Event::FileWritten { path, .. } => Some(path.as_path()),
            _ => None,
        })
        .collect();
    assert_eq!(
        writes,
        [project.join("src/calc.py"), project.join("new_file.txt")]
    );
}

#[test]
fn every_event_of_the_runs_names_its_session_terminal_process_folder_and_session_file() {
    let scratch = Scratch::new();
    let project = scratch.project();
    let records: Vec<PiRecord> = recorded::pi_events(TURNS)
        .iter()
        .map(|record| record.in_project(&project))
        .collect();
    let heard = replay(&records);

    assert!(!heard.is_empty());
    for (line, reported) in &heard {
        let record = &records[*line];
        assert_eq!(
            reported.session,
            CliSession(record.session().to_owned()),
            "line {line}"
        );
        assert_eq!(
            reported.terminal.as_deref(),
            record.terminal(),
            "line {line}"
        );
        assert_eq!(reported.pid, record.pid, "line {line}");
        assert_eq!(
            reported.folder.as_deref(),
            Some(project.as_path()),
            "line {line}"
        );
        assert_eq!(
            reported.cli_record.as_deref(),
            record.session_file().map(Path::new),
            "line {line}"
        );
    }
    assert!(records[1].terminal().is_some());
    assert!(records[1].session_file().is_some());
    assert_ne!(records[1].session(), records[129].session());
}

#[test]
fn no_line_of_any_recording_of_pi_gives_a_permission_event() {
    for name in RECORDINGS {
        let heard = replay(&recorded::pi_events(name));
        assert!(!heard.is_empty(), "{name} gives events");
        for (line, reported) in heard {
            assert!(
                !matches!(reported.event, Event::Permission { .. }),
                "line {line} of {name}"
            );
        }
    }
}

#[test]
fn a_prompt_naming_a_file_with_an_at_mention_gives_no_read() {
    let records = recorded::hand_made_pi_events(AT_MENTION);
    assert!(
        records
            .iter()
            .all(|record| !record.event.starts_with("tool_")),
        "the run holds no tool"
    );

    let heard = replay(&records);
    let events = vec![
        (0, Event::SessionStarted { how: Start::New }),
        (1, Event::TurnStarted),
        (3, finished()),
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
    let records = recorded::hand_made_pi_events(SHELL_READ);
    assert_eq!(records[3].event, "tool_call");
    assert_eq!(records[3].payload["toolName"], "bash");
    assert_eq!(records[3].payload["input"]["command"], "cat notes.txt");

    let heard = replay(&records);
    let events = vec![
        (0, Event::SessionStarted { how: Start::New }),
        (1, Event::TurnStarted),
        (3, began("cat notes.txt")),
        (4, ended("cat notes.txt")),
        (5, finished()),
        (6, Event::SessionEnded),
    ];
    assert_eq!(heard, expected(&records, events));
    assert!(
        heard
            .iter()
            .all(|(_, reported)| !matches!(reported.event, Event::FileRead { .. })),
        "a shell command names no file to the Adapter"
    );
}

fn say(adapter: &mut Pi, event: &str, fields: Value) -> Vec<Event> {
    try_say(adapter, event, fields)
        .expect("the payload is a report")
        .into_iter()
        .map(|reported| reported.event)
        .collect()
}

fn try_say(adapter: &mut Pi, event: &str, fields: Value) -> Result<Vec<Reported>, ReportError> {
    let mut payload = json!({
        "event": event,
        "sessionId": "s1",
        "sessionFile": "/sessions/s1.jsonl",
        "cwd": "/work",
    });
    merge(&mut payload, fields);
    adapter.report(&Report {
        terminal: Some("t1"),
        pid: 7,
        payload: &serde_json::to_vec(&payload).expect("a payload"),
    })
}

fn merge(into: &mut Value, fields: Value) {
    let (Value::Object(into), Value::Object(fields)) = (into, fields) else {
        panic!("objects");
    };
    into.extend(fields);
}

fn result(adapter: &mut Pi, tool: &str, input: Value, more: Value) -> Vec<Event> {
    let mut fields = json!({"toolName": tool, "input": input, "isError": false});
    merge(&mut fields, more);
    say(adapter, "tool_result", fields)
}

fn path_of(events: &[Event]) -> PathBuf {
    match events {
        [Event::FileRead { path, .. }] | [Event::FileWritten { path, .. }] => path.clone(),
        other => panic!("one file event, not {other:?}"),
    }
}

#[test]
fn a_read_with_offset_and_limit_names_its_line_range() {
    let mut adapter = Pi::new();
    let lines = |adapter: &mut Pi, input: Value| match result(adapter, "read", input, json!({}))[..]
    {
        [Event::FileRead { ref lines, .. }] => lines.clone(),
        ref other => panic!("one read event, not {other:?}"),
    };
    assert_eq!(
        lines(
            &mut adapter,
            json!({"path": "src/a.rs", "offset": 40, "limit": 81})
        ),
        Some(40..=120)
    );
    assert_eq!(
        lines(&mut adapter, json!({"path": "src/a.rs", "offset": 15})),
        Some(15..=15)
    );
    assert_eq!(
        lines(&mut adapter, json!({"path": "src/a.rs", "limit": 20})),
        Some(1..=20)
    );
    assert_eq!(
        lines(&mut adapter, json!({"path": "src/a.rs", "limit": 0})),
        None
    );
    assert_eq!(lines(&mut adapter, json!({"path": "src/a.rs"})), None);
}

#[test]
fn a_relative_path_is_joined_to_the_folder_and_an_absolute_one_is_kept_as_it_is() {
    let mut adapter = Pi::new();
    let read = |adapter: &mut Pi, path: &str| {
        path_of(&result(adapter, "read", json!({ "path": path }), json!({})))
    };
    assert_eq!(read(&mut adapter, "src/a.rs"), Path::new("/work/src/a.rs"));
    assert_eq!(read(&mut adapter, "/abs/b.rs"), Path::new("/abs/b.rs"));
    // Never cleaned up: the path is the CLI's own word for the file.
    assert_eq!(
        read(&mut adapter, "../up/./c.rs"),
        Path::new("/work/../up/./c.rs")
    );
    let written = result(&mut adapter, "write", json!({"path": "n.txt"}), json!({}));
    assert_eq!(path_of(&written), Path::new("/work/n.txt"));
}

#[test]
fn a_tool_that_failed_gives_no_file_event_but_a_command_that_failed_still_ended() {
    let mut adapter = Pi::new();
    for tool in ["read", "edit", "write"] {
        let heard = result(
            &mut adapter,
            tool,
            json!({"path": "a.txt"}),
            json!({"isError": true, "details": {"patch": "@@ -1 +1 @@\n-a\n+b\n"}}),
        );
        assert_eq!(heard, [], "a failed {tool}");
    }
    let heard = result(
        &mut adapter,
        "bash",
        json!({"command": "false"}),
        json!({"isError": true}),
    );
    assert_eq!(heard, [ended("false")]);
    assert_eq!(result(&mut adapter, "grep", json!({}), json!({})), []);
    assert_eq!(
        say(
            &mut adapter,
            "tool_call",
            json!({"toolName": "grep", "input": {}})
        ),
        []
    );
    assert_eq!(
        say(
            &mut adapter,
            "tool_call",
            json!({"toolName": "edit", "input": {"path": "a.txt"}})
        ),
        []
    );
}

#[test]
fn an_edit_gives_the_lines_of_every_hunk_of_its_patch_and_none_without_one() {
    let mut adapter = Pi::new();
    let lines = |adapter: &mut Pi, details: Value| match result(
        adapter,
        "edit",
        json!({"path": "a.txt"}),
        json!({"details": details}),
    )
    .as_slice()
    {
        [
            Event::FileWritten {
                tool: WriteTool::Edit,
                lines,
                ..
            },
        ] => lines.clone(),
        other => panic!("one edit, not {other:?}"),
    };
    // `+12` has no count: one line. `+22,0` took lines out and left none: the one line where they
    // were.
    let patch = "--- a.txt\n+++ a.txt\n@@ -1,2 +1,3 @@\n a\n+b\n c\n@@ -10 +12 @@\n-x\n+y\n@@ -20,2 +22,0 @@\n-p\n-q\n";
    assert_eq!(
        lines(
            &mut adapter,
            json!({ "patch": patch, "firstChangedLine": 99 })
        ),
        Some(vec![
            RangeInclusive::new(1, 3),
            RangeInclusive::new(12, 12),
            RangeInclusive::new(22, 22)
        ]),
        "firstChangedLine is not read"
    );
    // A hunk that took the first lines off names line 0 on the new side: lines count from 1.
    assert_eq!(
        lines(&mut adapter, json!({"patch": "@@ -1,2 +0,0 @@\n-a\n-b\n"})),
        Some(vec![RangeInclusive::new(1, 1)])
    );
    assert_eq!(lines(&mut adapter, json!(null)), None);
    assert_eq!(lines(&mut adapter, json!({"diff": "- 5 a\n+ 5 b"})), None);
}

#[test]
fn a_run_is_open_for_each_session_apart_and_ends_at_its_own_settling() {
    let mut adapter = Pi::new();
    let for_session = |adapter: &mut Pi, session: &str, event: &str, fields: Value| {
        let mut fields = fields;
        merge(&mut fields, json!({ "sessionId": session }));
        say(adapter, event, fields)
    };
    let input = json!({"source": "interactive", "streamingBehavior": null});

    assert_eq!(
        for_session(&mut adapter, "a", "input", input.clone()),
        [Event::TurnStarted]
    );
    assert_eq!(for_session(&mut adapter, "a", "agent_start", json!({})), []);
    assert_eq!(for_session(&mut adapter, "a", "input", input.clone()), []);
    assert_eq!(
        for_session(&mut adapter, "b", "agent_start", json!({})),
        [Event::TurnStarted]
    );
    assert_eq!(
        for_session(&mut adapter, "a", "agent_settled", json!({"aborted": true})),
        [Event::TurnEnded {
            how: TurnEnd::Interrupted
        }]
    );
    assert_eq!(
        for_session(&mut adapter, "a", "input", input.clone()),
        [Event::TurnStarted]
    );
    assert_eq!(for_session(&mut adapter, "b", "input", input.clone()), []);
    assert_eq!(
        for_session(
            &mut adapter,
            "b",
            "agent_settled",
            json!({"aborted": false})
        ),
        [finished()]
    );
}

#[test]
fn a_session_that_ends_forgets_its_open_run_and_a_reload_keeps_it() {
    let mut adapter = Pi::new();
    let input = json!({"source": "interactive", "streamingBehavior": null});
    assert_eq!(
        say(&mut adapter, "input", input.clone()),
        [Event::TurnStarted]
    );

    assert_eq!(
        say(
            &mut adapter,
            "session_shutdown",
            json!({"reason": "reload"})
        ),
        []
    );
    assert_eq!(
        say(&mut adapter, "session_start", json!({"reason": "reload"})),
        []
    );
    assert_eq!(say(&mut adapter, "input", input.clone()), []);

    // A quit with the run still open (pi killed) leaves no run for the session id a later resume
    // brings back.
    assert_eq!(
        say(&mut adapter, "session_shutdown", json!({"reason": "quit"})),
        [Event::SessionEnded]
    );
    assert_eq!(
        say(&mut adapter, "session_start", json!({"reason": "resume"})),
        [Event::SessionStarted {
            how: Start::Resumed
        }]
    );
    assert_eq!(say(&mut adapter, "input", input), [Event::TurnStarted]);

    for reason in ["startup", "new", "fork"] {
        assert_eq!(
            say(&mut adapter, "session_start", json!({ "reason": reason })),
            [Event::SessionStarted { how: Start::New }],
            "{reason}"
        );
    }
    for reason in ["quit", "new", "resume", "fork"] {
        assert_eq!(
            say(
                &mut adapter,
                "session_shutdown",
                json!({ "reason": reason })
            ),
            [Event::SessionEnded],
            "{reason}"
        );
    }
}

#[test]
fn a_session_file_of_none_gives_no_record_of_the_cli() {
    let mut adapter = Pi::new();
    let heard = try_say(
        &mut adapter,
        "session_start",
        json!({"reason": "startup", "sessionFile": null}),
    )
    .expect("a report");
    assert_eq!(heard.len(), 1);
    assert_eq!(heard[0].cli_record, None);
    assert_eq!(heard[0].folder.as_deref(), Some(Path::new("/work")));
    assert_eq!(heard[0].terminal.as_deref(), Some("t1"));
    assert_eq!(heard[0].pid, 7);
}

#[test]
fn a_payload_that_is_no_report_is_unreadable_and_one_of_an_event_it_does_not_follow_is_not() {
    let mut adapter = Pi::new();
    let report = |adapter: &mut Pi, payload: &[u8]| {
        adapter.report(&Report {
            terminal: None,
            pid: 1,
            payload,
        })
    };
    for payload in [
        &b"not json"[..],
        b"",
        b"[1, 2]",
        b"\"text\"",
        b"{}",
        br#"{"event": 3}"#,
        br#"{"event": "input", "source": "interactive"}"#,
        br#"{"event": "session_start", "reason": "startup", "sessionId": 4}"#,
        br#"{"event": "tool_call", "sessionId": "s", "toolName": "bash", "input": {}}"#,
        br#"{"event": "tool_result", "sessionId": "s", "toolName": "read", "input": {}, "isError": false}"#,
    ] {
        assert!(
            matches!(report(&mut adapter, payload), Err(ReportError::Unreadable(_))),
            "{}",
            String::from_utf8_lossy(payload)
        );
    }
    for payload in [
        &br#"{"event": "factory", "pid": 1}"#[..],
        br#"{"event": "inject:push", "route": "push"}"#,
        br#"{"event": "turn_start", "sessionId": "s"}"#,
        br#"{"event": "message_end", "sessionId": "s"}"#,
        br#"{"event": "before_agent_start", "sessionId": "s"}"#,
        br#"{"event": "resources_discover", "sessionId": "s"}"#,
    ] {
        assert_eq!(
            report(&mut adapter, payload).expect("no fault"),
            [],
            "{}",
            String::from_utf8_lossy(payload)
        );
    }
}

#[test]
fn a_call_of_the_reply_tool_gives_no_event() {
    let mut adapter = Pi::new();
    let heard = say(
        &mut adapter,
        REPLY_EVENT,
        json!({"toolCallId": "c1", "arguments": {"thread": "T1", "body": "done", "addressed": true}}),
    );
    assert_eq!(heard, []);
}

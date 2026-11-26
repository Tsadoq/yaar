use std::collections::HashMap;
use std::ops::RangeInclusive;
use std::path::PathBuf;

use serde_json::Value;
use yaar_agent::{
    CliSession, Event, Phase, Report, ReportError, Reported, Start, TurnEnd, WriteTool,
};

#[derive(Debug, Default)]
struct Open {
    terminal: Option<String>,
    pid: u32,
    folder: Option<PathBuf>,
    /// Whether a turn is open: begun by a prompt, not ended by a `Stop` or a mark.
    turn: bool,
    /// Whether the app was told the session waits for a permission, and not that it stopped.
    waiting: bool,
}

#[derive(Debug, Default)]
pub(crate) struct Remembered {
    sessions: HashMap<CliSession, Open>,
    terminals: HashMap<String, CliSession>,
}

impl Remembered {
    pub(crate) fn report(&mut self, report: &Report<'_>) -> Result<Vec<Reported>, ReportError> {
        let input: Value = serde_json::from_slice(report.payload)
            .map_err(|why| ReportError::Unreadable(why.to_string()))?;
        // A call of the reply tool is no hook: `asked` reads it, so it gives no event and no
        // error.
        if input.get("tool_server").is_some() {
            return Ok(Vec::new());
        }
        let hook = text(&input, "hook_event_name")
            .ok_or_else(|| unreadable("it names no hook_event_name"))?;
        let session = CliSession(
            text(&input, "session_id")
                .ok_or_else(|| unreadable("it names no session_id"))?
                .to_owned(),
        );
        // What the hook is about is read before anything is remembered, so that a report
        // that cannot be read changes nothing.
        let tool = tool_event(hook, &input)?;

        self.bind(&session, report, &input);
        let open = self.sessions.entry(session.clone()).or_default();

        let mut events = Vec::new();
        match hook {
            "SessionStart" => {
                let how = match text(&input, "source") {
                    // A compaction goes on inside the same turn.
                    Some("compact") => None,
                    Some("resume") => Some(Start::Resumed),
                    _ => Some(Start::New),
                };
                if let Some(how) = how {
                    open.turn = false;
                    open.waiting = false;
                    events.push(Event::SessionStarted { how });
                }
            }
            "UserPromptSubmit" => {
                open.turn = true;
                events.push(Event::TurnStarted);
            }
            "PermissionRequest" => {
                open.waiting = true;
                events.push(Event::Permission { waiting: true });
            }
            "PreToolUse" | "PostToolUse" | "PostToolUseFailure" => {
                let after = hook != "PreToolUse";
                if after && open.waiting {
                    open.waiting = false;
                    events.push(Event::Permission { waiting: false });
                }
                events.extend(tool);
            }
            "Stop" => {
                open.turn = false;
                events.push(Event::TurnEnded {
                    how: TurnEnd::Finished,
                });
            }
            "SessionEnd" => {
                events.push(Event::SessionEnded);
                self.forget(&session);
            }
            _ => {}
        }

        Ok(events
            .into_iter()
            .map(|event| Reported {
                session: session.clone(),
                terminal: report.terminal.map(str::to_owned),
                pid: report.pid,
                folder: text(&input, "cwd").map(PathBuf::from),
                cli_record: None,
                event,
            })
            .collect())
    }

    fn bind(&mut self, session: &CliSession, report: &Report<'_>, input: &Value) {
        let open = self.sessions.entry(session.clone()).or_default();
        open.pid = report.pid;
        if let Some(cwd) = text(input, "cwd") {
            open.folder = Some(PathBuf::from(cwd));
        }
        let Some(terminal) = report.terminal else {
            return;
        };
        let before = open.terminal.replace(terminal.to_owned());
        self.terminals.insert(terminal.to_owned(), session.clone());
        // A session that moves to another terminal, as a resume does, leaves the first.
        if let Some(before) = before.filter(|before| before != terminal)
            && self.terminals.get(&before) == Some(session)
        {
            self.terminals.remove(&before);
        }
    }

    /// Claude Code runs no hook for a turn stopped by Esc or a denied dialog; its mark then ends
    /// the open turn of the session that last reported from `terminal`.
    pub(crate) fn interrupted(&mut self, terminal: &str) -> Vec<Reported> {
        let Some(session) = self.terminals.get(terminal) else {
            return Vec::new();
        };
        let Some(open) = self.sessions.get_mut(session).filter(|open| open.turn) else {
            return Vec::new();
        };
        open.turn = false;

        let mut events = Vec::new();
        if open.waiting {
            open.waiting = false;
            events.push(Event::Permission { waiting: false });
        }
        events.push(Event::TurnEnded {
            how: TurnEnd::Interrupted,
        });
        events
            .into_iter()
            .map(|event| Reported {
                session: session.clone(),
                terminal: Some(terminal.to_owned()),
                pid: open.pid,
                folder: open.folder.clone(),
                cli_record: None,
                event,
            })
            .collect()
    }

    fn forget(&mut self, session: &CliSession) {
        if let Some(open) = self.sessions.remove(session)
            && let Some(terminal) = open.terminal
            && self.terminals.get(&terminal) == Some(session)
        {
            self.terminals.remove(&terminal);
        }
    }
}

fn tool_event(hook: &str, input: &Value) -> Result<Option<Event>, ReportError> {
    if !matches!(hook, "PreToolUse" | "PostToolUse" | "PostToolUseFailure") {
        return Ok(None);
    }
    let Some(tool) = text(input, "tool_name") else {
        return Ok(None);
    };
    let tool_input = &input["tool_input"];
    let command = || Event::Command {
        phase: if hook == "PreToolUse" {
            Phase::Began
        } else {
            Phase::Ended
        },
        command: text(tool_input, "command").unwrap_or_default().to_owned(),
    };
    // A tool that failed changed no file and read none, but a shell command that failed
    // has ended all the same.
    Ok(match (hook, tool) {
        (_, "Bash") => Some(command()),
        ("PostToolUse", "Read") => Some(Event::FileRead {
            path: path_of(tool, tool_input)?,
            lines: read_lines(tool_input),
        }),
        ("PostToolUse", "Write") => Some(Event::FileWritten {
            path: path_of(tool, tool_input)?,
            tool: WriteTool::Write,
            lines: None,
        }),
        ("PostToolUse", "Edit" | "MultiEdit" | "NotebookEdit") => Some(Event::FileWritten {
            path: path_of(tool, tool_input)?,
            tool: WriteTool::Edit,
            lines: patch_lines(&input["tool_response"]),
        }),
        _ => None,
    })
}

fn path_of(tool: &str, tool_input: &Value) -> Result<PathBuf, ReportError> {
    text(tool_input, "file_path")
        .or_else(|| text(tool_input, "notebook_path"))
        .map(PathBuf::from)
        .ok_or_else(|| unreadable(&format!("the input of {tool} names no file")))
}

/// The lines `Read` asked for, from `offset` and `limit` in `tool_input`. None when neither is
/// named or both are unusable.
fn read_lines(tool_input: &Value) -> Option<RangeInclusive<u32>> {
    let number = |key: &str| u32::try_from(tool_input.get(key)?.as_u64()?).ok();
    let offset = number("offset");
    let limit = number("limit").filter(|&limit| limit > 0);
    match (offset, limit) {
        (None, None) => None,
        (Some(start), Some(count)) => {
            let start = start.max(1);
            Some(start..=start.saturating_add(count - 1))
        }
        (Some(start), None) => {
            let start = start.max(1);
            Some(start..=start)
        }
        (None, Some(count)) => Some(1..=count),
    }
}

/// The lines an edit changed, from the hunks of `structuredPatch`. None where there is no
/// patch, no hunk or an unreadable hunk: a part of the lines would say less than was
/// changed.
fn patch_lines(response: &Value) -> Option<Vec<RangeInclusive<u32>>> {
    let hunks = response.get("structuredPatch")?.as_array()?;
    if hunks.is_empty() {
        return None;
    }
    hunks
        .iter()
        .map(|hunk| {
            // Lines count from 1; a patch of a file left empty starts at 0.
            let start = u32::try_from(hunk.get("newStart")?.as_u64()?).ok()?.max(1);
            let count = u32::try_from(hunk.get("newLines")?.as_u64()?).ok()?;
            Some(start..=start.checked_add(count.saturating_sub(1))?)
        })
        .collect()
}

pub(crate) fn text<'a>(object: &'a Value, key: &str) -> Option<&'a str> {
    object.get(key)?.as_str()
}

fn unreadable(why: &str) -> ReportError {
    ReportError::Unreadable(why.to_owned())
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use yaar_agent::Adapter;

    use super::*;
    use crate::ClaudeCode;

    fn hook(name: &str, more: Value) -> Value {
        let mut input = json!({"hook_event_name": name, "session_id": "s", "cwd": "/w"});
        for (key, value) in more.as_object().expect("an object") {
            input[key] = value.clone();
        }
        input
    }

    fn said(
        adapter: &mut ClaudeCode,
        terminal: Option<&str>,
        input: &Value,
    ) -> Result<Vec<Event>, ReportError> {
        let payload = serde_json::to_vec(input).expect("JSON is written");
        let reported = adapter.report(&Report {
            terminal,
            pid: 7,
            payload: &payload,
        })?;
        Ok(reported
            .into_iter()
            .map(|reported| reported.event)
            .collect())
    }

    fn edit_with(response: Value) -> Value {
        hook(
            "PostToolUse",
            json!({"tool_name": "Edit", "tool_input": {"file_path": "/w/f"}, "tool_response": response}),
        )
    }

    fn lines_of(response: Value) -> Option<Vec<RangeInclusive<u32>>> {
        match said(&mut ClaudeCode::new(), None, &edit_with(response)).expect("a report")[..] {
            [Event::FileWritten { ref lines, .. }] => lines.clone(),
            ref other => panic!("an edit gives one write, not {other:?}"),
        }
    }

    #[test]
    fn what_is_no_json_object_with_a_hook_and_a_session_cannot_be_read() {
        let mut adapter = ClaudeCode::new();
        for payload in [
            &b"not json"[..],
            b"[]",
            b"{}",
            br#"{"hook_event_name":"Stop"}"#,
        ] {
            let answer = adapter.report(&Report {
                terminal: None,
                pid: 1,
                payload,
            });
            assert!(
                matches!(answer, Err(ReportError::Unreadable(_))),
                "{:?}",
                String::from_utf8_lossy(payload)
            );
        }
        let answer = said(&mut adapter, None, &json!({"session_id": "s"}));
        assert!(matches!(answer, Err(ReportError::Unreadable(_))));
    }

    #[test]
    fn an_edit_gives_the_lines_each_hunk_left_in_the_file() {
        let patch = json!({"structuredPatch": [
            {"newStart": 3, "newLines": 2},
            {"newStart": 10, "newLines": 1},
        ]});

        assert_eq!(lines_of(patch), Some(vec![3..=4, 10..=10]));
    }

    #[test]
    fn a_hunk_that_left_no_line_names_the_one_where_it_was() {
        let patch = json!({"structuredPatch": [{"newStart": 9, "newLines": 0}]});

        assert_eq!(lines_of(patch), Some(vec![RangeInclusive::new(9, 9)]));
    }

    #[test]
    fn a_patch_of_a_file_left_empty_names_line_one() {
        let patch = json!({"structuredPatch": [{"newStart": 0, "newLines": 0}]});

        assert_eq!(lines_of(patch), Some(vec![RangeInclusive::new(1, 1)]));
    }

    #[test]
    fn an_edit_with_no_patch_or_an_unreadable_one_names_no_lines() {
        for response in [
            json!({}),
            json!("done"),
            json!({"structuredPatch": []}),
            json!({"structuredPatch": "x"}),
            json!({"structuredPatch": [{"newStart": 3, "newLines": 2}, {"newStart": "x"}]}),
            json!({"structuredPatch": [{"newStart": -1, "newLines": 2}]}),
            json!({"structuredPatch": [{"newStart": 4294967295u64, "newLines": 2}]}),
        ] {
            assert_eq!(lines_of(response.clone()), None, "{response}");
        }
    }

    #[test]
    fn a_notebook_edit_names_its_notebook() {
        let input = hook(
            "PostToolUse",
            json!({"tool_name": "NotebookEdit", "tool_input": {"notebook_path": "/w/n.ipynb"}}),
        );

        assert_eq!(
            said(&mut ClaudeCode::new(), None, &input).expect("a report"),
            [Event::FileWritten {
                path: PathBuf::from("/w/n.ipynb"),
                tool: WriteTool::Edit,
                lines: None
            }]
        );
    }

    #[test]
    fn a_tool_that_names_no_file_cannot_be_read_and_changes_nothing() {
        let mut adapter = ClaudeCode::new();
        said(&mut adapter, None, &hook("PermissionRequest", json!({}))).expect("a report");
        let unnamed = hook(
            "PostToolUse",
            json!({"tool_name": "Write", "tool_input": {}}),
        );

        assert!(matches!(
            said(&mut adapter, None, &unnamed),
            Err(ReportError::Unreadable(_))
        ));
        // The dialog is still open as far as the Adapter knows: the next answer ends it.
        let answer = hook("PostToolUse", json!({"tool_name": "Glob"}));
        assert_eq!(
            said(&mut adapter, None, &answer).expect("a report"),
            [Event::Permission { waiting: false }]
        );
    }

    #[test]
    fn a_failed_tool_ends_the_dialog_and_a_shell_command_but_changes_no_file() {
        for (tool, after) in [
            ("Bash", Some(Phase::Ended)),
            ("Edit", None),
            ("Write", None),
            ("Read", None),
        ] {
            let mut adapter = ClaudeCode::new();
            said(&mut adapter, None, &hook("PermissionRequest", json!({}))).expect("a report");
            let failed = hook(
                "PostToolUseFailure",
                json!({"tool_name": tool, "tool_input": {"command": "false", "file_path": "/w/f"}}),
            );

            let mut expected = vec![Event::Permission { waiting: false }];
            expected.extend(after.map(|phase| Event::Command {
                phase,
                command: "false".to_owned(),
            }));
            assert_eq!(
                said(&mut adapter, None, &failed).expect("a report"),
                expected,
                "{tool}"
            );
        }
    }

    #[test]
    fn a_tool_that_begins_while_a_dialog_is_open_leaves_waiting_on() {
        let mut adapter = ClaudeCode::new();
        said(&mut adapter, None, &hook("PermissionRequest", json!({}))).expect("a report");
        let begun = hook(
            "PreToolUse",
            json!({"tool_name": "Bash", "tool_input": {"command": "ls"}}),
        );
        let ended = hook(
            "PostToolUse",
            json!({"tool_name": "Bash", "tool_input": {"command": "ls"}}),
        );

        assert_eq!(
            said(&mut adapter, None, &begun).expect("a report"),
            [Event::Command {
                phase: Phase::Began,
                command: "ls".to_owned()
            }]
        );
        assert_eq!(
            said(&mut adapter, None, &ended).expect("a report"),
            [
                Event::Permission { waiting: false },
                Event::Command {
                    phase: Phase::Ended,
                    command: "ls".to_owned()
                }
            ]
        );
    }

    #[test]
    fn a_tool_that_follows_no_dialog_turns_no_permission_off() {
        let input = hook(
            "PostToolUse",
            json!({"tool_name": "Read", "tool_input": {"file_path": "/w/f"}}),
        );

        assert_eq!(
            said(&mut ClaudeCode::new(), None, &input).expect("a report"),
            [Event::FileRead {
                path: PathBuf::from("/w/f"),
                lines: None,
            }]
        );
    }

    #[test]
    fn a_read_with_offset_and_limit_names_its_line_range() {
        let read_with = |tool_input: Value| {
            let input = hook(
                "PostToolUse",
                json!({"tool_name": "Read", "tool_input": tool_input}),
            );
            match said(&mut ClaudeCode::new(), None, &input).expect("a report")[..] {
                [Event::FileRead { ref lines, .. }] => lines.clone(),
                ref other => panic!("a read gives one event, not {other:?}"),
            }
        };

        assert_eq!(
            read_with(json!({"file_path": "/w/f", "offset": 40, "limit": 81})),
            Some(40..=120)
        );
        assert_eq!(
            read_with(json!({"file_path": "/w/f", "offset": 25})),
            Some(25..=25)
        );
        assert_eq!(
            read_with(json!({"file_path": "/w/f", "limit": 50})),
            Some(1..=50)
        );
        assert_eq!(
            read_with(json!({"file_path": "/w/f", "offset": 0, "limit": 10})),
            Some(1..=10)
        );
        assert_eq!(read_with(json!({"file_path": "/w/f", "limit": 0})), None);
        assert_eq!(read_with(json!({"file_path": "/w/f"})), None);
    }

    #[test]
    fn each_dialog_of_a_turn_turns_waiting_on() {
        let mut adapter = ClaudeCode::new();
        let dialog = hook("PermissionRequest", json!({}));

        for _ in 0..2 {
            assert_eq!(
                said(&mut adapter, None, &dialog).expect("a report"),
                [Event::Permission { waiting: true }]
            );
        }
    }

    #[test]
    fn a_session_start_says_how_it_started_and_a_compaction_says_nothing() {
        for (source, how) in [
            ("startup", Some(Start::New)),
            ("clear", Some(Start::New)),
            ("resume", Some(Start::Resumed)),
            ("compact", None),
        ] {
            let input = hook("SessionStart", json!({ "source": source }));

            assert_eq!(
                said(&mut ClaudeCode::new(), None, &input).expect("a report"),
                how.map(|how| Event::SessionStarted { how })
                    .into_iter()
                    .collect::<Vec<_>>(),
                "{source}"
            );
        }
    }

    #[test]
    fn a_compaction_does_not_end_the_turn_it_happens_in() {
        let mut adapter = ClaudeCode::new();
        said(
            &mut adapter,
            Some("t"),
            &hook("UserPromptSubmit", json!({})),
        )
        .expect("a report");
        said(
            &mut adapter,
            Some("t"),
            &hook("SessionStart", json!({"source": "compact"})),
        )
        .expect("a report");

        assert_eq!(adapter.marked("t", 0).len(), 1);
    }

    #[test]
    fn a_hook_the_app_does_not_follow_gives_nothing() {
        for name in [
            "InstructionsLoaded",
            "PostToolBatch",
            "Notification",
            "SubagentStart",
            "SubagentStop",
            "A hook of a later version",
        ] {
            assert_eq!(
                said(&mut ClaudeCode::new(), None, &hook(name, json!({}))).expect("a report"),
                [],
                "{name}"
            );
        }
    }

    #[test]
    fn the_pre_hook_of_a_tool_that_is_no_shell_gives_nothing() {
        let input = hook(
            "PreToolUse",
            json!({"tool_name": "Write", "tool_input": {"file_path": "/w/f"}}),
        );

        assert_eq!(
            said(&mut ClaudeCode::new(), None, &input).expect("a report"),
            []
        );
    }

    #[test]
    fn a_report_names_where_it_came_from_and_the_folder_the_session_works_in() {
        let mut adapter = ClaudeCode::new();
        let input = hook("UserPromptSubmit", json!({}));
        let payload = serde_json::to_vec(&input).expect("JSON is written");

        let reported = adapter
            .report(&Report {
                terminal: Some("t1"),
                pid: 42,
                payload: &payload,
            })
            .expect("a report");

        assert_eq!(reported.len(), 1);
        assert_eq!(reported[0].session, CliSession("s".to_owned()));
        assert_eq!(reported[0].terminal.as_deref(), Some("t1"));
        assert_eq!(reported[0].pid, 42);
        assert_eq!(reported[0].folder, Some(PathBuf::from("/w")));
    }

    #[test]
    fn a_report_with_no_cwd_names_no_folder() {
        let mut input = hook("UserPromptSubmit", json!({}));
        input.as_object_mut().expect("an object").remove("cwd");
        let payload = serde_json::to_vec(&input).expect("JSON is written");

        let reported = ClaudeCode::new()
            .report(&Report {
                terminal: None,
                pid: 1,
                payload: &payload,
            })
            .expect("a report");

        assert_eq!(reported[0].folder, None);
        assert_eq!(reported[0].terminal, None);
    }

    #[test]
    fn the_mark_is_told_with_the_process_and_folder_of_the_last_hook() {
        let mut adapter = ClaudeCode::new();
        let input = hook("UserPromptSubmit", json!({"cwd": "/elsewhere"}));
        let payload = serde_json::to_vec(&input).expect("JSON is written");
        adapter
            .report(&Report {
                terminal: Some("t1"),
                pid: 42,
                payload: &payload,
            })
            .expect("a report");

        let reported = adapter.marked("t1", 0);

        assert_eq!(reported.len(), 1);
        assert_eq!(reported[0].pid, 42);
        assert_eq!(reported[0].folder, Some(PathBuf::from("/elsewhere")));
        assert_eq!(reported[0].terminal.as_deref(), Some("t1"));
        assert_eq!(reported[0].session, CliSession("s".to_owned()));
    }

    #[test]
    fn a_mark_in_a_terminal_ends_the_turn_of_the_session_that_runs_there() {
        let mut adapter = ClaudeCode::new();
        let mut other = hook("UserPromptSubmit", json!({}));
        other["session_id"] = json!("other");
        said(
            &mut adapter,
            Some("a"),
            &hook("UserPromptSubmit", json!({})),
        )
        .expect("a report");
        said(&mut adapter, Some("b"), &other).expect("a report");

        let reported = adapter.marked("b", 0);

        assert_eq!(reported.len(), 1);
        assert_eq!(reported[0].session, CliSession("other".to_owned()));
        assert_eq!(adapter.marked("a", 0).len(), 1);
    }

    #[test]
    fn a_session_that_moves_to_another_terminal_leaves_the_first() {
        let mut adapter = ClaudeCode::new();
        said(
            &mut adapter,
            Some("a"),
            &hook("SessionStart", json!({"source": "startup"})),
        )
        .expect("a report");
        said(
            &mut adapter,
            Some("b"),
            &hook("UserPromptSubmit", json!({})),
        )
        .expect("a report");

        assert_eq!(adapter.marked("a", 0), []);
        assert_eq!(adapter.marked("b", 0).len(), 1);
    }

    #[test]
    fn a_session_that_ended_is_forgotten() {
        let mut adapter = ClaudeCode::new();
        said(
            &mut adapter,
            Some("t"),
            &hook("UserPromptSubmit", json!({})),
        )
        .expect("a report");
        said(&mut adapter, Some("t"), &hook("SessionEnd", json!({}))).expect("a report");

        assert_eq!(adapter.marked("t", 0), []);
    }

    #[test]
    fn a_report_that_cannot_be_read_changes_nothing() {
        let mut adapter = ClaudeCode::new();
        said(
            &mut adapter,
            Some("t"),
            &hook("UserPromptSubmit", json!({})),
        )
        .expect("a report");
        let unreadable = hook(
            "PostToolUse",
            json!({"tool_name": "Write", "tool_input": {}}),
        );

        assert!(said(&mut adapter, Some("elsewhere"), &unreadable).is_err());

        assert_eq!(adapter.marked("elsewhere", 0), [], "it bound no terminal");
        assert_eq!(adapter.marked("t", 0).len(), 1, "the turn is open still");
    }
}

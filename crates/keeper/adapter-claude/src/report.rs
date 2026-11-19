//! The reading of one hook's input, and what the Adapter remembers between hooks.

use std::collections::HashMap;
use std::ops::RangeInclusive;
use std::path::PathBuf;

use serde_json::Value;
use yaar_agent::{
    CliSession, Event, Phase, Report, ReportError, Reported, Start, TurnEnd, WriteTool,
};

/// What the Adapter knows of one session between two hooks.
#[derive(Debug, Default)]
struct Open {
    /// The terminal it last reported from.
    terminal: Option<String>,
    /// Its process, as the last report named it.
    pid: u32,
    /// Where it works, as the last report named it.
    folder: Option<PathBuf>,
    /// Whether a turn is open: begun by a prompt, not ended by a `Stop` or a mark.
    turn: bool,
    /// Whether the app was told that the session waits for a permission, and not that it
    /// stopped.
    waiting: bool,
}

/// Every session the Adapter has heard of, and the session that last reported from each
/// terminal.
#[derive(Debug, Default)]
pub(crate) struct Remembered {
    sessions: HashMap<CliSession, Open>,
    terminals: HashMap<String, CliSession>,
}

impl Remembered {
    /// The events of one hook's input.
    pub(crate) fn report(&mut self, report: &Report<'_>) -> Result<Vec<Reported>, ReportError> {
        let input: Value = serde_json::from_slice(report.payload)
            .map_err(|why| ReportError::Unreadable(why.to_string()))?;
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
                event,
            })
            .collect())
    }

    /// Notes where the session reports from: its process, its folder and its terminal, which
    /// from now on is the terminal's session.
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

    /// What the mark that Claude Code writes when a turn stops by Esc, or by a dialog
    /// denied, means for the session that last reported from `terminal`.
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
                event,
            })
            .collect()
    }

    /// Drops a session that ended, and the terminal's memory of it.
    fn forget(&mut self, session: &CliSession) {
        if let Some(open) = self.sessions.remove(session)
            && let Some(terminal) = open.terminal
            && self.terminals.get(&terminal) == Some(session)
        {
            self.terminals.remove(&terminal);
        }
    }
}

/// The event a tool's hook gives of its own, apart from the permission it may end: none
/// for a hook that is no tool's, or a tool the app does not follow.
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

/// The file a tool names, as the CLI wrote it.
fn path_of(tool: &str, tool_input: &Value) -> Result<PathBuf, ReportError> {
    text(tool_input, "file_path")
        .or_else(|| text(tool_input, "notebook_path"))
        .map(PathBuf::from)
        .ok_or_else(|| unreadable(&format!("the input of {tool} names no file")))
}

/// The lines of the file that an edit changed, from the hunks of its `structuredPatch`:
/// each from `newStart` for `newLines`, or the one line at `newStart` where it left none.
/// None where there is no patch, no hunk, or a hunk that cannot be read: a part of the
/// lines would say less than was changed.
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

/// The string `key` holds, if it holds one.
fn text<'a>(object: &'a Value, key: &str) -> Option<&'a str> {
    object.get(key)?.as_str()
}

fn unreadable(why: &str) -> ReportError {
    ReportError::Unreadable(why.to_owned())
}

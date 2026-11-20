//! What pi's extension reports, read as the app's events, what a report asks, and what the
//! Adapter answers: the tables of the crate's doc.

use std::collections::HashSet;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};
use yaar_agent::{
    AgentReply, Answer, Asked, Asks, CliSession, Event, Phase, Report, ReportError, Reported,
    Start, TurnEnd, WriteTool,
};

use crate::REPLY_EVENT;

/// What the Adapter remembers between reports: for each session, by pi's id of it, whether
/// a run of its agent is open, so that a message pi queues while it works starts no turn.
#[derive(Debug, Default)]
pub(crate) struct Remembered {
    /// pi's session ids whose run is open.
    open: HashSet<String>,
}

/// One payload, read as far as every report needs: the object, and its `event`.
struct Payload {
    fields: Map<String, Value>,
}

impl Payload {
    /// The payload of `report`: a JSON object that names its `event`.
    fn of(report: &Report<'_>) -> Result<Payload, ReportError> {
        let value: Value = serde_json::from_slice(report.payload)
            .map_err(|e| unreadable(format!("the payload is not JSON: {e}")))?;
        let Value::Object(fields) = value else {
            return Err(unreadable("the payload is not a JSON object".to_owned()));
        };
        if fields.get("event").and_then(Value::as_str).is_none() {
            return Err(unreadable("the payload names no event".to_owned()));
        }
        Ok(Payload { fields })
    }

    fn event(&self) -> &str {
        self.text("event").unwrap_or_default()
    }

    /// pi's id of the session the event is of. Every event the crate's doc names holds one.
    fn session(&self) -> Result<&str, ReportError> {
        self.text("sessionId")
            .ok_or_else(|| unreadable(format!("{} names no session", self.event())))
    }

    /// The text under `key`, none where it is missing or is not text.
    fn text(&self, key: &str) -> Option<&str> {
        self.fields.get(key).and_then(Value::as_str)
    }

    /// The folder pi works in, `cwd`.
    fn folder(&self) -> Option<PathBuf> {
        self.text("cwd").map(PathBuf::from)
    }

    /// The tool of a `tool_call` or a `tool_result`.
    fn tool(&self) -> Result<&str, ReportError> {
        self.text("toolName")
            .ok_or_else(|| unreadable(format!("{} names no tool", self.event())))
    }

    /// The text `key` of the tool's input, as `path` and `command`.
    fn input_text(&self, key: &str) -> Result<&str, ReportError> {
        self.fields
            .get("input")
            .and_then(|input| input.get(key))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                unreadable(format!(
                    "{} of {} names no {key}",
                    self.event(),
                    self.text("toolName").unwrap_or("a tool")
                ))
            })
    }

    /// Whether the tool failed.
    fn failed(&self) -> bool {
        self.fields
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// The file a tool names, as the app gives it: absolute, a relative one joined to the
    /// folder pi works in, and never cleaned up.
    fn file(&self) -> Result<PathBuf, ReportError> {
        let named = Path::new(self.input_text("path")?);
        Ok(match self.folder() {
            Some(folder) => folder.join(named),
            None => named.to_path_buf(),
        })
    }
}

fn unreadable(why: String) -> ReportError {
    ReportError::Unreadable(why)
}

/// The lines of the file as it is now that a unified diff changed: for each hunk header
/// `@@ -a,b +c,d @@`, `c` to `c + d - 1`, or `c` alone where `d` is 0 (the lines were taken
/// out and none put in), the first line being 1. None where the diff holds no hunk.
fn lines_of(patch: &str) -> Option<Vec<RangeInclusive<u32>>> {
    let lines: Vec<_> = patch.lines().filter_map(hunk).collect();
    (!lines.is_empty()).then_some(lines)
}

/// The lines the new side of one hunk header names; none for a line that is no header.
fn hunk(line: &str) -> Option<RangeInclusive<u32>> {
    let (_old, rest) = line.strip_prefix("@@ -")?.split_once(" +")?;
    let new = rest.split(' ').next()?;
    let (start, count) = match new.split_once(',') {
        Some((start, count)) => (start.parse::<u32>().ok()?, count.parse::<u32>().ok()?),
        None => (new.parse::<u32>().ok()?, 1),
    };
    // A hunk that took lines off the top names line 0: the file's first line is 1.
    let start = start.max(1);
    Some(start..=start.saturating_add(count.saturating_sub(1)))
}

impl Remembered {
    /// The events of one report, as the crate's doc says. A payload that is no JSON object,
    /// or names no `event`, is [`ReportError::Unreadable`]; one whose `event` the crate doc's
    /// table does not name gives no event and no error, whatever else it holds (a recording's
    /// `factory` line names no session); one the table names with no `sessionId` is
    /// [`ReportError::Unreadable`].
    pub(crate) fn report(&mut self, report: &Report<'_>) -> Result<Vec<Reported>, ReportError> {
        let payload = Payload::of(report)?;
        if !is_named(payload.event()) {
            return Ok(Vec::new());
        }
        let session = payload.session()?;
        let events = self.events(&payload, session)?;
        Ok(events
            .into_iter()
            .map(|event| Reported {
                session: CliSession(session.to_owned()),
                terminal: report.terminal.map(str::to_owned),
                pid: report.pid,
                folder: payload.folder(),
                cli_record: payload.text("sessionFile").map(PathBuf::from),
                event,
            })
            .collect())
    }

    /// The events of a payload of the session `session`, and what the Adapter remembers of
    /// the session after it.
    fn events(&mut self, payload: &Payload, session: &str) -> Result<Vec<Event>, ReportError> {
        let reason = payload.text("reason");
        Ok(match payload.event() {
            "session_start" => match reason {
                Some("reload") => Vec::new(),
                Some("resume") => vec![Event::SessionStarted {
                    how: Start::Resumed,
                }],
                _ => vec![Event::SessionStarted { how: Start::New }],
            },
            "session_shutdown" => match reason {
                Some("reload") => Vec::new(),
                _ => {
                    // A session pi brings back under its id, after a resume or a restart,
                    // starts with no run open.
                    self.open.remove(session);
                    vec![Event::SessionEnded]
                }
            },
            "input" | "agent_start" => {
                if self.open.insert(session.to_owned()) {
                    vec![Event::TurnStarted]
                } else {
                    Vec::new()
                }
            }
            "agent_settled" => {
                self.open.remove(session);
                let aborted = payload
                    .fields
                    .get("aborted")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let how = if aborted {
                    TurnEnd::Interrupted
                } else {
                    TurnEnd::Finished
                };
                vec![Event::TurnEnded { how }]
            }
            "tool_call" => match payload.tool()? {
                "bash" => vec![command(Phase::Began, payload)?],
                _ => Vec::new(),
            },
            "tool_result" => match payload.tool()? {
                "bash" => vec![command(Phase::Ended, payload)?],
                _ if payload.failed() => Vec::new(),
                "read" => vec![Event::FileRead {
                    path: payload.file()?,
                }],
                "edit" => vec![Event::FileWritten {
                    path: payload.file()?,
                    tool: WriteTool::Edit,
                    lines: payload
                        .fields
                        .get("details")
                        .and_then(|details| details.get("patch"))
                        .and_then(Value::as_str)
                        .and_then(lines_of),
                }],
                "write" => vec![Event::FileWritten {
                    path: payload.file()?,
                    tool: WriteTool::Write,
                    lines: None,
                }],
                _ => Vec::new(),
            },
            // A call of the reply tool gives no event, only what it asks.
            _ => Vec::new(),
        })
    }

    /// What one report asks: see the crate's doc. It remembers nothing; `folder` is not
    /// read, since pi's delivery writes no file.
    pub(crate) fn asked(
        &self,
        folder: &Path,
        report: &Report<'_>,
    ) -> Result<Option<Asked>, ReportError> {
        let _ = folder;
        let payload = Payload::of(report)?;
        let asks = match payload.event() {
            // A message an extension sent carries no review; the user's does, whether pi
            // is idle or queues it.
            "input" if payload.text("source") != Some("extension") => Asks::Message,
            REPLY_EVENT => Asks::Reply(reply(&payload)?),
            _ => return Ok(None),
        };
        Ok(Some(Asked {
            session: CliSession(payload.session()?.to_owned()),
            terminal: report.terminal.map(str::to_owned),
            folder: payload.folder(),
            asks,
        }))
    }

    /// The answer to a call of the reply tool, as bytes the extension reads:
    /// `{"text": "recorded reply to <thread> (addressed|open, <n> characters)"}` for
    /// [`Answer::Replied`], `{"error": <why>}` for [`Answer::Refused`], and nothing for
    /// [`Answer::Allow`], which pi never asks for.
    pub(crate) fn answer(&self, asked: &Asked, answer: &Answer<'_>) -> Vec<u8> {
        let _ = asked;
        let said = match answer {
            Answer::Allow => return Vec::new(),
            Answer::Replied {
                thread,
                addressed,
                characters,
            } => {
                let state = if *addressed { "addressed" } else { "open" };
                json!({"text": format!("recorded reply to {thread} ({state}, {characters} characters)")})
            }
            Answer::Refused { why } => json!({ "error": why }),
        };
        // A JSON value is always written.
        serde_json::to_vec(&said).unwrap_or_default()
    }
}

/// Whether the crate's doc names the event: the ones the Adapter reads.
fn is_named(event: &str) -> bool {
    matches!(
        event,
        "session_start"
            | "session_shutdown"
            | "input"
            | "agent_start"
            | "tool_call"
            | "tool_result"
            | "agent_settled"
            | REPLY_EVENT
    )
}

/// The step of a shell command, as the agent wrote the command.
fn command(phase: Phase, payload: &Payload) -> Result<Event, ReportError> {
    Ok(Event::Command {
        phase,
        command: payload.input_text("command")?.to_owned(),
    })
}

/// What the agent said to a thread, from the `arguments` of a call of the reply tool.
fn reply(payload: &Payload) -> Result<AgentReply, ReportError> {
    let arguments = payload
        .fields
        .get("arguments")
        .and_then(Value::as_object)
        .ok_or_else(|| unreadable("a reply holds no arguments".to_owned()))?;
    let text = |key: &str| {
        arguments
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| unreadable(format!("a reply names no {key}")))
    };
    let addressed = match arguments.get("addressed") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(addressed)) => *addressed,
        Some(_) => {
            return Err(unreadable(
                "`addressed` of a reply is not a boolean".to_owned(),
            ));
        }
    };
    Ok(AgentReply {
        thread: text("thread")?,
        text: text("body")?,
        addressed,
    })
}

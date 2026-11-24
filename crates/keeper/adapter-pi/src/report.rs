use std::collections::HashSet;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};
use yaar_agent::{
    AgentReply, Answer, Asked, Asks, CliSession, Event, Phase, Report, ReportError, Reported,
    Start, TurnEnd, WriteTool,
};

use crate::REPLY_EVENT;

/// Per session: whether a run of the agent is open, so that a message pi queues while it works
/// starts no turn.
#[derive(Debug, Default)]
pub(crate) struct Remembered {
    open: HashSet<String>,
}

struct Payload {
    fields: Map<String, Value>,
}

impl Payload {
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

    fn session(&self) -> Result<&str, ReportError> {
        self.text("sessionId")
            .ok_or_else(|| unreadable(format!("{} names no session", self.event())))
    }

    fn text(&self, key: &str) -> Option<&str> {
        self.fields.get(key).and_then(Value::as_str)
    }

    fn folder(&self) -> Option<PathBuf> {
        self.text("cwd").map(PathBuf::from)
    }

    fn tool(&self) -> Result<&str, ReportError> {
        self.text("toolName")
            .ok_or_else(|| unreadable(format!("{} names no tool", self.event())))
    }

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

    fn failed(&self) -> bool {
        self.fields
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// The file a tool names: a relative path is joined to `cwd` and never cleaned up.
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

/// Lines of the file after a unified diff: per hunk `@@ -a,b +c,d @@`, `c` to `c + d - 1`, or `c`
/// alone when `d` is 0 (lines only taken out). None where the diff holds no hunk.
fn lines_of(patch: &str) -> Option<Vec<RangeInclusive<u32>>> {
    let lines: Vec<_> = patch.lines().filter_map(hunk).collect();
    (!lines.is_empty()).then_some(lines)
}

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
    /// A payload that is no JSON object or names no `event` is `Unreadable`, and so is a known
    /// `event` with no `sessionId`; an `event` outside the known set gives nothing.
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
                    // A session pi brings back under its id (resume, restart) starts with no run
                    // open.
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
            _ => Vec::new(),
        })
    }

    pub(crate) fn asked(&self, report: &Report<'_>) -> Result<Option<Asked>, ReportError> {
        let payload = Payload::of(report)?;
        let asks = match payload.event() {
            // A message an extension sent carries no review; the user's does, idle or queued.
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

    pub(crate) fn answer(&self, answer: &Answer<'_>) -> Vec<u8> {
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
        serde_json::to_vec(&said).unwrap_or_default()
    }
}

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

fn command(phase: Phase, payload: &Payload) -> Result<Event, ReportError> {
    Ok(Event::Command {
        phase,
        command: payload.input_text("command")?.to_owned(),
    })
}

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

//! Recorded events of the real agent CLIs (Claude Code 2.1.323, pi 1.1.0), replayed by the tests.
//! Their hooks, plugin and server are named `ade`; `RENAMED` maps those names to the app's.

use std::collections::BTreeMap;
use std::path::PathBuf;

const FOLDER: &str = "recorded";

const HAND_MADE: &str = "hand-made";

pub const PROJECT: &str = "/home/reviewer/project";

/// Names the recordings carry and the app's names read in their place; an empty name drops a
/// variable only the recording harness read.
const RENAMED: [(&str, &str); 11] = [
    ("ADE_TERM_ID", yaar_link::TERMINAL_VARIABLE),
    ("ADE_HOOK_LOG", ""),
    ("ADE_X", ""),
    ("ADE_RUN", ""),
    ("ADE_REPLY_LOG", ""),
    ("ADE_INJECT_FILE", ""),
    ("ADE_INJECT_MODE", ""),
    ("ADE_ALLOW_DIR", ""),
    (
        "mcp__plugin_ade-full_ade__",
        "mcp__plugin_yaar-review_yaar__",
    ),
    ("mcp__plugin_ade_ade__", "mcp__plugin_yaar-review_yaar__"),
    ("plugin:ade-full:ade", "plugin:yaar-review:yaar"),
];

#[derive(Debug, Clone, PartialEq)]
pub struct HookRecord {
    pub event: String,
    pub input: serde_json::Value,
    pub env: BTreeMap<String, String>,
    pub pid: u32,
    pub parent: u32,
    pub started_ns: u64,
}

impl HookRecord {
    /// Equal in meaning to the recorded input, not byte for byte: object keys come out ordered by
    /// name.
    pub fn input_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.input).expect("a JSON value is written")
    }

    pub fn in_project(&self, root: &std::path::Path) -> HookRecord {
        let root = root.to_string_lossy();
        let mut record = self.clone();
        in_project_of(&mut record.input, &root);
        for setting in record.env.values_mut() {
            *setting = moved(setting, &root);
        }
        record
    }

    pub fn terminal(&self) -> Option<&str> {
        self.env
            .get(yaar_link::TERMINAL_VARIABLE)
            .map(String::as_str)
    }

    pub fn session(&self) -> &str {
        self.input["session_id"].as_str().unwrap_or_default()
    }
}

fn path(name: &str) -> PathBuf {
    let path = folder().join(name);
    assert!(
        path.is_file(),
        "the recording {name} is not at {}: every test that replays a recorded event of \
         the agent CLIs needs its file there",
        path.display()
    );
    path
}

fn hand_made(name: &str) -> PathBuf {
    let path = folder().join(HAND_MADE).join(name);
    assert!(
        path.is_file(),
        "the hand-made shape {name} is not at {}",
        path.display()
    );
    path
}

/// The non-empty lines of a hand-made shape after the first, which must say why it was written
/// by hand.
fn hand_made_records(name: &str) -> Vec<String> {
    let path = hand_made(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("the hand-made shape {} cannot be read: {e}", path.display()));
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let says_why = lines
        .next()
        .and_then(|first| serde_json::from_str::<serde_json::Value>(first).ok())
        .is_some_and(|first| {
            first["hand_made"]
                .as_str()
                .is_some_and(|why| !why.is_empty())
        });
    assert!(
        says_why,
        "the first line of the hand-made shape {name} does not say, under `hand_made`, why it was written by hand"
    );
    lines.map(str::to_owned).collect()
}

pub fn hand_made_hook_events(name: &str) -> Vec<HookRecord> {
    hand_made_records(name)
        .iter()
        .enumerate()
        .map(|(number, line)| {
            hook_record(line).unwrap_or_else(|| {
                panic!(
                    "line {} of the hand-made shape {name} is not a hook's record",
                    number + 1
                )
            })
        })
        .collect()
}

pub fn hook_events(name: &str) -> Vec<HookRecord> {
    lines(name)
        .iter()
        .enumerate()
        .map(|(number, line)| {
            hook_record(line).unwrap_or_else(|| {
                panic!("line {number} of the recording {name} is not a hook's record")
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnswerShape {
    pub answers: String,
    /// Its texts are placeholders: a test compares its keys and the kinds of their values.
    pub stdout: serde_json::Value,
}

pub fn hand_made_answers(name: &str) -> Vec<AnswerShape> {
    hand_made_records(name)
        .iter()
        .enumerate()
        .map(|(number, line)| {
            let value: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|e| {
                panic!(
                    "line {} of the hand-made shape {name} is not JSON: {e}",
                    number + 1
                )
            });
            match (value["answers"].as_str(), value.get("stdout")) {
                (Some(answers), Some(stdout)) => AnswerShape {
                    answers: answers.to_owned(),
                    stdout: stdout.clone(),
                },
                _ => panic!(
                    "line {} of the hand-made shape {name} is no answer's shape",
                    number + 1
                ),
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
}

#[derive(Debug, Clone, PartialEq)]
pub struct McpMessage {
    pub direction: Direction,
    pub message: serde_json::Value,
}

/// The `Out` lines are the recorded server's own answers, not the app's: a test sends the `In`
/// messages to the app's server and asserts facts of what that server answers.
pub fn mcp_messages(name: &str) -> Vec<McpMessage> {
    lines(name)
        .iter()
        .enumerate()
        .map(|(number, line)| {
            let value: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|e| {
                panic!("line {number} of the recording {name} is not JSON: {e}")
            });
            let direction = match value["dir"].as_str() {
                Some("in") => Direction::In,
                Some("out") => Direction::Out,
                _ => panic!("line {number} of the recording {name} goes no known way"),
            };
            McpMessage {
                direction,
                message: value["msg"].clone(),
            }
        })
        .collect()
}

pub fn terminal_chunks(name: &str) -> Vec<Vec<u8>> {
    lines(name)
        .iter()
        .enumerate()
        .map(|(number, line)| {
            let value: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|e| {
                panic!("line {number} of the recording {name} is not JSON: {e}")
            });
            value["d"]
                .as_str()
                .unwrap_or_else(|| {
                    panic!("line {number} of the recording {name} holds no terminal bytes")
                })
                .as_bytes()
                .to_vec()
        })
        .collect()
}

/// One recorded event of pi, as the recording's logging extension wrote it. Lines whose `event`
/// starts with `inject:` are the recorder's own, never pi's.
#[derive(Debug, Clone, PartialEq)]
pub struct PiRecord {
    pub event: String,
    pub payload: serde_json::Value,
    pub pid: u32,
}

impl PiRecord {
    pub fn payload_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.payload).expect("a JSON value is written")
    }

    /// A relative `input.path` stays relative: it is relative to `cwd`, which moves.
    pub fn in_project(&self, root: &std::path::Path) -> PiRecord {
        let root = root.to_string_lossy();
        let mut record = self.clone();
        in_project_of(&mut record.payload, &root);
        record
    }

    pub fn terminal(&self) -> Option<&str> {
        self.payload[yaar_link::TERMINAL_VARIABLE].as_str()
    }

    pub fn session(&self) -> &str {
        self.payload["sessionId"].as_str().unwrap_or_default()
    }

    pub fn session_file(&self) -> Option<&str> {
        self.payload["sessionFile"].as_str()
    }
}

pub fn pi_events(name: &str) -> Vec<PiRecord> {
    lines(name)
        .iter()
        .enumerate()
        .map(|(number, line)| {
            pi_record(line).unwrap_or_else(|| {
                panic!("line {number} of the recording {name} is not a record of pi's")
            })
        })
        .collect()
}

pub fn hand_made_pi_events(name: &str) -> Vec<PiRecord> {
    hand_made_records(name)
        .iter()
        .enumerate()
        .map(|(number, line)| {
            pi_record(line).unwrap_or_else(|| {
                panic!(
                    "line {} of the hand-made shape {name} is not a record of pi's",
                    number + 1
                )
            })
        })
        .collect()
}

fn pi_record(line: &str) -> Option<PiRecord> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let event = value.get("event")?.as_str()?.to_owned();
    let pid = u32::try_from(value.get("pid")?.as_u64()?).ok()?;
    let mut fields = serde_json::Map::new();
    for (name, field) in value.as_object()? {
        match RENAMED.iter().find(|(from, _)| from == name) {
            Some((_, "")) => {}
            Some((_, to)) => {
                fields.insert((*to).to_owned(), field.clone());
            }
            None => {
                fields.insert(name.clone(), field.clone());
            }
        }
    }
    Some(PiRecord {
        event,
        payload: serde_json::Value::Object(fields),
        pid,
    })
}

fn folder() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FOLDER)
}

fn lines(name: &str) -> Vec<String> {
    let path = path(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("the recording {} cannot be read: {e}", path.display()))
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .collect()
}

fn hook_record(line: &str) -> Option<HookRecord> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let mut input = value.get("stdin")?.clone();
    rename_in(&mut input);
    let mut env = BTreeMap::new();
    for (name, setting) in value.get("env")?.as_object()? {
        let name = match RENAMED.iter().find(|(from, _)| from == name) {
            Some((_, "")) => continue,
            Some((_, to)) => (*to).to_owned(),
            None => name.clone(),
        };
        env.insert(name, setting.as_str()?.to_owned());
    }
    Some(HookRecord {
        event: value.get("event")?.as_str()?.to_owned(),
        input,
        env,
        pid: u32::try_from(value.get("pid")?.as_u64()?).ok()?,
        parent: u32::try_from(value.get("ppid")?.as_u64()?).ok()?,
        started_ns: value.get("t_start_ns")?.as_str()?.parse().ok()?,
    })
}

fn in_project_of(value: &mut serde_json::Value, root: &str) {
    match value {
        serde_json::Value::String(text) => *text = moved(text, root),
        serde_json::Value::Array(items) => {
            items.iter_mut().for_each(|item| in_project_of(item, root))
        }
        serde_json::Value::Object(fields) => fields
            .values_mut()
            .for_each(|field| in_project_of(field, root)),
        _ => {}
    }
}

fn moved(text: &str, root: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(PROJECT) {
        let after = &rest[at + PROJECT.len()..];
        out.push_str(&rest[..at]);
        let continues_a_name =
            after.starts_with(|c: char| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'));
        out.push_str(if continues_a_name { PROJECT } else { root });
        rest = after;
    }
    out.push_str(rest);
    out
}

fn rename_in(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => {
            // The names of tools and servers, which are parts of a text; the variables are keys,
            // renamed where the environment is read.
            for (from, to) in RENAMED
                .iter()
                .filter(|(from, _)| from.starts_with("mcp__") || from.starts_with("plugin:"))
            {
                if text.contains(from) {
                    *text = text.replace(from, to);
                }
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(rename_in),
        serde_json::Value::Object(fields) => fields.values_mut().for_each(rename_in),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_of_pi_is_read_with_the_variable_of_the_terminal_under_the_apps_name() {
        let line = r#"{"ts": "x", "pid": 50199, "event": "tool_call", "sessionId": "s-1", "sessionFile": "/home/reviewer/experiment/pi/sess/a.jsonl", "cwd": "/home/reviewer/project", "ADE_TERM_ID": "term-7a", "toolName": "read", "input": {"path": "src/calc.py"}}"#;
        let record = pi_record(line).expect("a record of pi's");
        assert_eq!(record.event, "tool_call");
        assert_eq!(record.pid, 50199);
        assert_eq!(record.terminal(), Some("term-7a"));
        assert!(record.payload.get("ADE_TERM_ID").is_none());
        assert_eq!(record.session(), "s-1");
        assert_eq!(
            record.session_file(),
            Some("/home/reviewer/experiment/pi/sess/a.jsonl")
        );
        let moved = record.in_project(std::path::Path::new("/tmp/w"));
        assert_eq!(moved.payload["cwd"], "/tmp/w");
        assert_eq!(moved.payload["input"]["path"], "src/calc.py");
        let again: serde_json::Value =
            serde_json::from_slice(&moved.payload_bytes()).expect("the payload is JSON");
        assert_eq!(again, moved.payload);
        assert!(pi_record(r#"{"event": "input"}"#).is_none(), "no pid");
        assert!(pi_record("not json").is_none());
    }

    #[test]
    fn every_line_of_the_recordings_of_pi_is_a_record_and_names_a_session_but_the_factorys() {
        for name in [
            "pi/turn-tools.jsonl",
            "pi/resume-quit-abort.jsonl",
            "pi/review-before-agent-start.jsonl",
            "pi/queued-message.jsonl",
            "pi/reply-tool.jsonl",
        ] {
            for (number, record) in pi_events(name).iter().enumerate() {
                let own = record.event == "factory" || record.event.starts_with("inject:");
                assert!(
                    own || !record.session().is_empty(),
                    "line {number} of {name} names no session"
                );
            }
        }
    }

    #[test]
    fn the_shapes_of_pi_written_by_hand_say_so_first_and_then_hold_records_of_pi() {
        for name in [
            "pi-shell-read.jsonl",
            "pi-at-mention.jsonl",
            "pi-typed-while-working.jsonl",
        ] {
            let records = hand_made_pi_events(name);
            assert!(!records.is_empty(), "{name} holds no record");
            assert!(records.iter().all(|record| !record.session().is_empty()));
        }
    }

    #[test]
    fn a_hook_line_is_read_with_the_working_names_mapped_to_the_apps() {
        let line = r#"{"env": {"ADE_HOOK_LOG": "/x/hooks.jsonl", "ADE_TERM_ID": "term-a", "CLAUDE_PID": "7"}, "event": "PreToolUse", "pid": 8, "ppid": 7, "t_start_ns": "1779564123349216000", "stdin": {"session_id": "s-1", "tool_name": "mcp__plugin_ade-full_ade__reply_to_thread", "tool_input": {"note": "mcp__plugin_ade_ade__reply_to_thread"}}}"#;
        let record = hook_record(line).expect("a hook's record");
        assert_eq!(record.event, "PreToolUse");
        assert_eq!((record.pid, record.parent), (8, 7));
        assert_eq!(record.started_ns, 1_779_564_123_349_216_000);
        assert_eq!(record.session(), "s-1");
        assert_eq!(record.terminal(), Some("term-a"));
        assert!(!record.env.contains_key("ADE_HOOK_LOG"));
        assert!(!record.env.contains_key("ADE_TERM_ID"));
        assert_eq!(record.env.get("CLAUDE_PID").map(String::as_str), Some("7"));
        assert_eq!(
            record.input["tool_name"],
            "mcp__plugin_yaar-review_yaar__reply_to_thread"
        );
        assert_eq!(
            record.input["tool_input"]["note"],
            "mcp__plugin_yaar-review_yaar__reply_to_thread"
        );
    }

    #[test]
    fn a_record_moved_into_another_folder_names_that_folder_wherever_it_named_the_project() {
        let line = r#"{"env": {"CLAUDE_PROJECT_DIR": "/home/reviewer/project"}, "event": "PreToolUse", "pid": 8, "ppid": 7, "t_start_ns": "5", "stdin": {"cwd": "/home/reviewer/project", "tool_input": {"file_path": "/home/reviewer/project/a1.txt", "command": "echo A3 > /home/reviewer/project/a3.txt && cat /home/reviewer/project2/x"}}}"#;
        let record = hook_record(line)
            .expect("a hook's record")
            .in_project(std::path::Path::new("/tmp/w 1/repo"));
        assert_eq!(record.input["cwd"], "/tmp/w 1/repo");
        assert_eq!(
            record.input["tool_input"]["file_path"],
            "/tmp/w 1/repo/a1.txt"
        );
        assert_eq!(
            record.input["tool_input"]["command"],
            "echo A3 > /tmp/w 1/repo/a3.txt && cat /home/reviewer/project2/x"
        );
        assert_eq!(
            record.env.get("CLAUDE_PROJECT_DIR").map(String::as_str),
            Some("/tmp/w 1/repo")
        );
    }

    #[test]
    fn a_line_that_is_no_hooks_record_is_none() {
        assert_eq!(hook_record(r#"{"t": "1", "d": "x"}"#), None);
        assert_eq!(
            hook_record(r#"{"env": {}, "event": "Stop", "pid": 8, "ppid": 7, "stdin": {}}"#),
            None
        );
        assert_eq!(hook_record("not json"), None);
    }

    #[test]
    fn a_shape_written_by_hand_says_so_first_and_then_holds_hooks() {
        let records = hand_made_hook_events("claude-shell-read.jsonl");
        let events: Vec<&str> = records.iter().map(|record| record.event.as_str()).collect();
        assert_eq!(
            events,
            [
                "SessionStart",
                "UserPromptSubmit",
                "PreToolUse",
                "PostToolUse",
                "Stop",
                "SessionEnd"
            ]
        );
        assert!(
            records
                .iter()
                .all(|record| record.session() == records[0].session())
        );
        assert_eq!(records[2].input["tool_name"], "Bash");
        assert_eq!(records[2].input["cwd"], PROJECT);
    }

    #[test]
    fn the_working_names_of_the_reply_tool_and_its_server_are_mapped_and_the_tools_variables_dropped()
     {
        let line = r#"{"env": {"ADE_RUN": "r", "ADE_REPLY_LOG": "/x", "ADE_INJECT_FILE": "/y", "ADE_INJECT_MODE": "json", "ADE_ALLOW_DIR": "/z", "CLAUDE_PID": "7"}, "event": "PreToolUse", "pid": 8, "ppid": 7, "t_start_ns": "5", "stdin": {"tool_name": "mcp__plugin_ade-full_ade__reply_to_thread", "mcp_server": {"name": "plugin:ade-full:ade", "source": "plugin"}}}"#;
        let record = hook_record(line).expect("a hook's record");
        assert_eq!(record.env.keys().collect::<Vec<_>>(), ["CLAUDE_PID"]);
        assert_eq!(
            record.input["tool_name"],
            "mcp__plugin_yaar-review_yaar__reply_to_thread"
        );
        assert_eq!(
            record.input["mcp_server"]["name"],
            "plugin:yaar-review:yaar"
        );
    }

    #[test]
    fn the_shapes_of_two_answers_written_by_hand_say_so_first() {
        let answers = hand_made_answers("claude-hook-answers.jsonl");
        let hooks: Vec<&str> = answers.iter().map(|shape| shape.answers.as_str()).collect();
        assert_eq!(hooks, ["UserPromptSubmit", "PreToolUse"]);
        assert!(answers[0].stdout["hookSpecificOutput"]["additionalContext"].is_string());
        assert_eq!(
            answers[1].stdout["hookSpecificOutput"]["permissionDecision"],
            "allow"
        );
    }

    #[test]
    fn a_typed_message_after_one_claude_code_started_itself_is_a_shape_written_by_hand() {
        let records = hand_made_hook_events("claude-typed-after-self-started.jsonl");
        let prompts: Vec<bool> = records
            .iter()
            .filter(|record| record.event == "UserPromptSubmit")
            .map(|record| {
                record.input["prompt"]
                    .as_str()
                    .expect("a prompt")
                    .starts_with("<task-notification>")
            })
            .collect();
        assert_eq!(prompts, [false, true, false]);
    }

    #[test]
    fn the_trace_of_the_tool_server_goes_both_ways_in_order() {
        let messages = mcp_messages("claude/reply-tool-mcp.jsonl");
        assert_eq!(messages.len(), 17);
        assert_eq!(messages[2].direction, Direction::In);
        assert_eq!(messages[2].message["method"], "initialize");
        assert_eq!(messages[3].direction, Direction::Out);
        assert_eq!(messages[7].message["method"], "tools/call");
        assert_eq!(messages[7].message["params"]["name"], "reply_to_thread");
    }

    #[test]
    #[should_panic(expected = "the recording claude/no-such-recording.jsonl is not at")]
    fn a_recording_that_is_not_there_fails_naming_it() {
        path("claude/no-such-recording.jsonl");
    }
}

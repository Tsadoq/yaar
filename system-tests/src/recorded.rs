//! The recorded events of the real agent CLIs, as the tests of the Adapters, the Companion
//! and the Workspaces replay them.
//!
//! The recordings were made on a Mac with Claude Code 2.1.323 and pi 1.1.0, cleaned, and
//! read by the user, who allowed them into the repository. They lie in [`FOLDER`] of this
//! crate, as `claude/<name>.jsonl` and `pi/<name>.jsonl`, each line one event. A test
//! that reads a file that is not there fails, naming the file it wanted: no test passes
//! or skips silently for lack of its recording.
//!
//! Every recording was made in one project folder, [`PROJECT`]: a test that replays one
//! in a worktree of its own reads it through [`HookRecord::in_project`].
//!
//! Where a row needs a shape that was never recorded, its test reads a file of
//! [`HAND_MADE`] instead: a shape written by hand, which says so in its name and in its
//! first line, and which is an ordinary file of the repository.
//!
//! The recordings carry the working name of the app from before it was called YAAR, `ade`,
//! in the names of the variables of the experiment's hooks and of its plugin and tool.
//! The files stay as the user read them; [`hook_events`] maps those names to the app's
//! as it reads them, by the one table [`RENAMED`], so that a test sees what the app's own
//! pieces would have seen.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// The folder of the recordings, inside this crate's folder: `system-tests/recorded`.
pub const FOLDER: &str = "recorded";

/// The folder of the shapes written by hand, inside [`FOLDER`]: `recorded/hand-made`.
pub const HAND_MADE: &str = "hand-made";

/// The project folder every recording was made in, as its paths name it.
pub const PROJECT: &str = "/home/reviewer/project";

/// The working names the recordings carry, and the app's names a test reads in their
/// place: the variable that named the terminal in the experiment's shells and the app's,
/// and the names of the experiment's plugin and tool server and the app's, the plugin
/// `yaar-review` and its server `yaar`, as a tool's name (`mcp__plugin_<plugin>_<server>__`)
/// and as the server's name in a hook's input (`plugin:<plugin>:<server>`). A variable that
/// only the experiment's tools read is dropped, which the empty name says.
///
/// It covers every working name a test reads: the hooks' inputs and the variables of
/// `claude/*.jsonl`, and the key that names the terminal in each line of `pi/*.jsonl`. The
/// recordings of pi carry the working name in values no test reads as well: the paths of
/// the experiment's extensions in the arguments of each `factory` line, the custom type
/// and the tag of the experiment's review in the heads of messages, and the experiment's
/// own lines, whose `event` starts with `inject:`. The folders of the experiment's plugins (`ade-full`, `ade-d`,
/// `ade-d-inline`, `ade-full-inline`) stay in the values of variables no test reads,
/// `CLAUDE_CODE_PLUGIN_DIRS`, `CLAUDE_PLUGIN_ROOT` and `CLAUDE_PLUGIN_DATA`, and in the
/// text of the experiment's own review inside a tool's response, which no test reads
/// either; and the tool server's trace, `claude/reply-tool-mcp.jsonl`, names the
/// experiment's server `ade` only in what the server answered, which [`mcp_messages`] gives
/// as it was and no test compares with the app's.
pub const RENAMED: [(&str, &str); 11] = [
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

/// One recorded hook of Claude Code: what the hook program received, and the environment
/// it ran in.
#[derive(Debug, Clone, PartialEq)]
pub struct HookRecord {
    /// The name of the hook, as `PermissionRequest`.
    pub event: String,
    /// The hook's input, as Claude Code handed it on standard input, with the names of
    /// [`RENAMED`] mapped.
    pub input: serde_json::Value,
    /// The variables the hook process saw, with the names of [`RENAMED`] mapped and the
    /// dropped ones left out.
    pub env: BTreeMap<String, String>,
    /// The hook process.
    pub pid: u32,
    /// Its parent: Claude Code.
    pub parent: u32,
    /// When the hook process started, in nanoseconds of the recording's clock: the gap
    /// between two records is the gap between the two hooks, which a replay keeps.
    pub started_ns: u64,
}

impl HookRecord {
    /// The input as a hook program would read it: the JSON written again from
    /// [`HookRecord::input`], equal in meaning to what was recorded but not byte for byte,
    /// since the keys of an object come out ordered by name.
    pub fn input_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.input).expect("a JSON value is written")
    }

    /// The same record as if it had been made in the folder `root`: wherever a text of
    /// the input or of the environment names [`PROJECT`] or a path under it, as `cwd`,
    /// `tool_input.file_path` and the paths a shell command names, `root` takes its
    /// place. A replay in a worktree of the test's own reads its records
    /// through this, so that the paths the Adapter gives lie in that worktree.
    pub fn in_project(&self, root: &std::path::Path) -> HookRecord {
        let root = root.to_string_lossy();
        let mut record = self.clone();
        in_project_of(&mut record.input, &root);
        for setting in record.env.values_mut() {
            *setting = moved(setting, &root);
        }
        record
    }

    /// The terminal its environment names, under the app's variable.
    pub fn terminal(&self) -> Option<&str> {
        self.env
            .get(yaar_link::TERMINAL_VARIABLE)
            .map(String::as_str)
    }

    /// The CLI's own session, `session_id` of the input.
    pub fn session(&self) -> &str {
        self.input["session_id"].as_str().unwrap_or_default()
    }
}

/// The path of the recording `name`, as `claude/permission-dialogs.jsonl`. Panics, naming
/// the file and saying where it is wanted, when it is not there.
pub fn path(name: &str) -> PathBuf {
    let path = folder().join(name);
    assert!(
        path.is_file(),
        "the recording {name} is not at {}: every test that replays a recorded event of \
         the agent CLIs needs its file there",
        path.display()
    );
    path
}

/// The path of the shape written by hand `name`, in [`HAND_MADE`]. Panics, naming it,
/// when it is not there.
pub fn hand_made(name: &str) -> PathBuf {
    let path = folder().join(HAND_MADE).join(name);
    assert!(
        path.is_file(),
        "the hand-made shape {name} is not at {}",
        path.display()
    );
    path
}

/// Every hook of the shape written by hand `name`, in [`HAND_MADE`], in the order of its
/// lines: the first line, which says why the shape was written by hand, is no hook and is
/// left out. Panics as [`hand_made`] does, when the first line does not say why, and on a
/// later line that is not a hook's record, naming its number.
pub fn hand_made_hook_events(name: &str) -> Vec<HookRecord> {
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
    lines
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

/// Every hook of the recording `name`, in the order of its lines, with the names of
/// [`RENAMED`] mapped. Every test with recorded events of Claude Code's hooks reads them
/// here. Panics as [`path`] does, and on a line that is not a hook's record, naming its
/// number.
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

/// One answer of a hook as Claude Code reads it, in the shapes written by hand: what a
/// hook of the event `answers` prints on its standard output.
#[derive(Debug, Clone, PartialEq)]
pub struct AnswerShape {
    /// The hook whose answer it is, as `UserPromptSubmit`.
    pub answers: String,
    /// The JSON the hook prints. Its texts are placeholders: a test compares its keys and
    /// the kinds of their values.
    pub stdout: serde_json::Value,
}

/// The answers of the shape written by hand `name`, in [`HAND_MADE`], in the order of its
/// lines, the first, which says why, left out. Panics as [`hand_made_hook_events`] does,
/// and on a later line that is no answer's shape, naming its number.
pub fn hand_made_answers(name: &str) -> Vec<AnswerShape> {
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
    lines
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

/// Which way a message of a tool server's trace went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// From Claude Code to the server.
    In,
    /// From the server to Claude Code.
    Out,
}

/// One message of the trace of a tool server, as `claude/reply-tool-mcp.jsonl` holds it:
/// what Claude Code sent the experiment's server, and what it answered.
#[derive(Debug, Clone, PartialEq)]
pub struct McpMessage {
    /// Which way it went.
    pub direction: Direction,
    /// The JSON-RPC message, as it was.
    pub message: serde_json::Value,
}

/// Every message of the tool server's trace `name`, in the order of its lines. A test that
/// plays Claude Code to the app's tool server sends the messages that went [`Direction::In`]
/// and names facts of what it reads back: the answers the experiment's server gave are its
/// own, not the app's. Panics as [`path`] does, and on a line that is no message of a
/// trace, naming its number.
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

/// The chunks of a terminal's output in the recording `name`, as
/// `claude/interrupt-mark.jsonl` holds them, each as the bytes the program wrote.
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

/// One recorded event of pi, as the experiment's logging extension wrote it: what pi gave
/// the extension, with the session and the folder of the extension's context.
///
/// A line is one JSON object whose `event` names pi's event and whose other keys are what
/// pi gave with it, under pi's own names (`sessionId`, `sessionFile`, `cwd`, `reason`,
/// `source`, `streamingBehavior`, `toolName`, `input`, `isError`, `details`, `aborted`):
/// the shape the app's extension reports in, as the crate doc of `yaar-adapter-pi` says, so
/// that [`PiRecord::payload_bytes`] is a payload a test hands the Adapter. The logger's own
/// keys (`ts`, `tNs`, `keys`, `mode`, and the lengths and heads of texts) are there too,
/// and the Adapter reads none of them. The lines the experiment's extension wrote of its own
/// doing, whose `event` starts with `inject:`, are records of the experiment: a test names
/// them as facts of the recording, never as pi's.
#[derive(Debug, Clone, PartialEq)]
pub struct PiRecord {
    /// pi's event, as `tool_result`; or the experiment's own, as `inject:transform`.
    pub event: String,
    /// The whole line, with the names of [`RENAMED`] mapped: the variable that named the
    /// terminal is under the app's name.
    pub payload: serde_json::Value,
    /// pi's process.
    pub pid: u32,
}

impl PiRecord {
    /// The line as the app's extension would write it as a report's payload: the JSON of
    /// [`PiRecord::payload`], equal in meaning to the recorded line, its keys ordered by name.
    pub fn payload_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.payload).expect("a JSON value is written")
    }

    /// The same record as if it had been made in the folder `root`: every text that names
    /// [`PROJECT`] or a path under it names `root` in its place, as
    /// [`HookRecord::in_project`] does. A relative `input.path` stays relative: it is
    /// relative to `cwd`, which moves.
    pub fn in_project(&self, root: &std::path::Path) -> PiRecord {
        let root = root.to_string_lossy();
        let mut record = self.clone();
        in_project_of(&mut record.payload, &root);
        record
    }

    /// The terminal it names, under the app's variable.
    pub fn terminal(&self) -> Option<&str> {
        self.payload[yaar_link::TERMINAL_VARIABLE].as_str()
    }

    /// pi's own id of the session, `sessionId`.
    pub fn session(&self) -> &str {
        self.payload["sessionId"].as_str().unwrap_or_default()
    }

    /// The file pi keeps the session in, `sessionFile`; none where it names none.
    pub fn session_file(&self) -> Option<&str> {
        self.payload["sessionFile"].as_str()
    }
}

/// Every event of pi's recording `name`, as `pi/turn-tools.jsonl`, in the order of its
/// lines, with the names of [`RENAMED`] mapped. Every test with recorded events of pi reads
/// them here. Panics as [`path`] does, and on a line that is not a record of pi's, naming
/// its number.
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

/// Every event of the shape of pi's written by hand `name`, in [`HAND_MADE`], in the order of
/// its lines, the first, which says why, left out. Panics as [`hand_made_hook_events`] does,
/// and on a later line that is not a record of pi's, naming its number.
pub fn hand_made_pi_events(name: &str) -> Vec<PiRecord> {
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
    lines
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

/// One line of pi's recording, with the names of [`RENAMED`] mapped; none for a line that is
/// not one: no JSON object, or no `event` or `pid`.
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

/// The folder of the recordings in this checkout.
fn folder() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FOLDER)
}

/// The lines of a recording, each one event.
fn lines(name: &str) -> Vec<String> {
    let path = path(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("the recording {} cannot be read: {e}", path.display()))
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .collect()
}

/// One line of a hook's recording, with the names of [`RENAMED`] mapped; none for a line
/// that is not one.
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

/// Every text of `value` with [`PROJECT`] replaced by `root`, as [`HookRecord::in_project`]
/// says.
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

/// `text` with every [`PROJECT`] that is the folder itself or a path under it replaced by
/// `root`: a name that merely starts with it, as `/home/reviewer/project2`, stays.
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

/// Maps the working names of [`RENAMED`] that are parts of a text, as the name of a tool,
/// in every text of `value`.
fn rename_in(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => {
            // The names of tools and servers, which are parts of a text; the variables are
            // keys, renamed where the environment is read.
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
        // A hook's record with no time of its own is no record a replay can pace.
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

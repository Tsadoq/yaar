//! The recorded events of the real agent CLIs, as the tests of the Adapters, the Companion
//! and the Workspaces replay them.
//!
//! The recordings were made on a Mac with Claude Code 2.1.323 and pi 1.1.0, cleaned, and
//! read by the user before they may enter the repository. They lie in [`FOLDER`] of this
//! crate, as `claude/<name>.jsonl` and `pi/<name>.jsonl`, each line one event. Until the
//! user has said yes, the files are kept out of git by the repository's own exclude file,
//! and a machine that has none of them fails every test that reads one, naming the file
//! it wanted: no test passes or skips silently for lack of its recording.
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

/// The working names the recordings carry, and the app's names a test reads in their
/// place: the variable that named the terminal in the experiment's shells and the app's,
/// and the names of the experiment's plugin and tool server and the app's. A variable
/// that only the experiment's logger read is dropped, which the empty name says.
pub const RENAMED: [(&str, &str); 5] = [
    ("ADE_TERM_ID", yaar_link::TERMINAL_VARIABLE),
    ("ADE_HOOK_LOG", ""),
    ("ADE_X", ""),
    ("mcp__plugin_ade-full_ade__", "mcp__plugin_yaar_yaar__"),
    ("mcp__plugin_ade_ade__", "mcp__plugin_yaar_yaar__"),
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
}

impl HookRecord {
    /// The input as the bytes a hook program reads.
    pub fn input_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.input).expect("a JSON value is written")
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
        "the recording {name} is not at {}: the recorded events of the agent CLIs are kept \
         out of git until they may enter it, and every test that replays one needs them \
         there",
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
    })
}

/// Maps the working names of [`RENAMED`] that are parts of a text, as the name of a tool,
/// in every text of `value`.
fn rename_in(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => {
            for (from, to) in RENAMED.iter().filter(|(from, _)| from.starts_with("mcp__")) {
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
    fn a_hook_line_is_read_with_the_working_names_mapped_to_the_apps() {
        let line = r#"{"env": {"ADE_HOOK_LOG": "/x/hooks.jsonl", "ADE_TERM_ID": "term-a", "CLAUDE_PID": "7"}, "event": "PreToolUse", "pid": 8, "ppid": 7, "stdin": {"session_id": "s-1", "tool_name": "mcp__plugin_ade-full_ade__reply_to_thread", "tool_input": {"note": "mcp__plugin_ade_ade__reply_to_thread"}}}"#;
        let record = hook_record(line).expect("a hook's record");
        assert_eq!(record.event, "PreToolUse");
        assert_eq!((record.pid, record.parent), (8, 7));
        assert_eq!(record.session(), "s-1");
        assert_eq!(record.terminal(), Some("term-a"));
        assert!(!record.env.contains_key("ADE_HOOK_LOG"));
        assert!(!record.env.contains_key("ADE_TERM_ID"));
        assert_eq!(record.env.get("CLAUDE_PID").map(String::as_str), Some("7"));
        assert_eq!(
            record.input["tool_name"],
            "mcp__plugin_yaar_yaar__reply_to_thread"
        );
        assert_eq!(
            record.input["tool_input"]["note"],
            "mcp__plugin_yaar_yaar__reply_to_thread"
        );
    }

    #[test]
    fn a_line_that_is_no_hooks_record_is_none() {
        assert_eq!(hook_record(r#"{"t": "1", "d": "x"}"#), None);
        assert_eq!(hook_record("not json"), None);
    }

    #[test]
    #[should_panic(expected = "the recording claude/no-such-recording.jsonl is not at")]
    fn a_recording_that_is_not_there_fails_naming_it() {
        path("claude/no-such-recording.jsonl");
    }
}

//! The delivery module of Claude Code: how a review rides with the user's message, what
//! the hooks answer, and what the reply tool's server is answered with.
//!
//! A review rides in the answer of the `UserPromptSubmit` hook: inline, as
//! `hookSpecificOutput.additionalContext`, while the text is at most [`INLINE_LIMIT`]
//! UTF-16 code units, Claude Code's limit for one string of a hook's output; else written
//! whole to `<folder>/`[`REVIEWS`]`/<CLI session id>/<review id>-<round>.md`, the
//! Adapter's own folder of the app, outside every repository, through a temporary file
//! renamed into place. The answer then holds a pointer, well under the limit: the text of
//! [`INSTRUCTION`], then the file's absolute path and the request to read it whole. Either
//! answer also holds a `systemMessage`, the one line the user sees under the prompt,
//! `yaar: review round <n> attached (<k> threads)`, `k` the threads the rendered review
//! holds ([`yaar_agent::Rendered::threads`]); the `yaar: ` before it says whose line it
//! is.
//!
//! The answers are JSON objects as Claude Code reads them from a hook's standard output:
//!
//! ```json
//! {"hookSpecificOutput": {"hookEventName": "UserPromptSubmit", "additionalContext": "<text>"},
//!  "systemMessage": "<line>"}
//! ```
//!
//! ```json
//! {"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "allow",
//!  "permissionDecisionReason": "<why>"}}
//! ```
//!
//! The second is given only for a `Read` whose `file_path`, made canonical, lies inside the
//! folder of reviews of the session that reports it, `<folder>/`[`REVIEWS`]`/<its id>/`
//! made canonical, and for a call of [`LISTED_REPLY_TOOL`]: the two things the user allowed
//! by installing the app. The rule needs nothing remembered, so it holds after the Keeper
//! started again. A `PermissionRequest`, and every other hook, is answered with nothing.
//!
//! A CLI session id is a folder name only when it is made of ASCII letters, digits, `-`
//! and `_`, as Claude Code's are; a review for a session of any other id is not written,
//! and its delivery fails, so nothing is sent.
//!
//! A call of the reply tool reaches the Adapter as a report of the tool server, whose
//! payload is a JSON object the server writes, `{"tool_server": "yaar", "session_id":
//! "<CLAUDE_CODE_SESSION_ID>", "cwd": "<its folder>", "name": "reply_to_thread",
//! "arguments": {...}}`; its answer is the `result` of the MCP call, as the server hands it
//! to Claude Code: `{"content": [{"type": "text", "text": "recorded reply to <thread>
//! (addressed|open, <n> characters)"}]}`, or with `"isError": true` and the reason when the
//! reply was refused.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use yaar_agent::{
    AgentReply, Answer, Asked, Asks, CliSession, Delivery, Rendered, Report, ReportError,
};

use crate::COMMAND;
use crate::report::Remembered;

/// The longest text that rides inline in a hook's answer, in UTF-16 code units, as Claude
/// Code counts a string: 10,000 arrive whole, 10,001 are replaced by a path and a preview.
pub const INLINE_LIMIT: usize = 10_000;

/// The folder, inside the Adapter's own folder of the app, that holds the reviews too
/// long to ride inline: a folder for each CLI session, named by its id, and in it one file
/// for each round, `<review id>-<round>.md`.
pub const REVIEWS: &str = "reviews";

/// The tool server's name in the plugin's `.mcp.json`.
pub const TOOL_SERVER_NAME: &str = "yaar";

/// The program of the app that serves the reply tool to Claude Code over its standard
/// input and output, in the folder of the app's programs beside the hook program.
pub const TOOL_SERVER: &str = "yaar-claude-tools";

/// The reply tool, as the tool server lists it.
pub const REPLY_TOOL: &str = "reply_to_thread";

/// The reply tool, as Claude Code lists it to the agent: `mcp__plugin_<plugin>_<server>__`
/// and its name.
pub const LISTED_REPLY_TOOL: &str = "mcp__plugin_yaar-review_yaar__reply_to_thread";

/// The instruction that goes first in every review delivered to Claude Code: how to answer
/// each thread with the reply tool, and nothing that resolves one.
pub const INSTRUCTION: &str = "This is a code review of your changes, attached to the \
user's message. Address each thread below. For each thread, call the tool \
mcp__plugin_yaar-review_yaar__reply_to_thread once, with `thread` set to the id written \
after \"Thread\" in its heading, `body` set to your reply to the reviewer, and \
`addressed` set to true when your change addresses the comment, or false when you \
disagree or need the reviewer's answer. Only the reviewer resolves a thread.";

/// What a message Claude Code starts itself opens with, as a background task that finished
/// reports: the user did not type it.
const SELF_STARTED: &str = "<task-notification>";

impl Remembered {
    /// What a report waits for: see [`yaar_agent::Adapter::asked`]. A `UserPromptSubmit`
    /// whose prompt opens, after white space, with `<task-notification>` is a message
    /// Claude Code started itself and asks nothing; any other prompt, whatever its words,
    /// is [`yaar_agent::Asks::Message`]. A `PreToolUse` of `Read` of a file inside the
    /// folder of reviews of that session under `folder` (this module's doc) is
    /// [`yaar_agent::Asks::ReadOwnFile`], one of [`LISTED_REPLY_TOOL`] is
    /// [`yaar_agent::Asks::OwnTool`]. A report of the tool server is
    /// [`yaar_agent::Asks::Reply`]. A `PermissionRequest`, and every other hook, asks
    /// nothing. It remembers nothing: the Keeper has given `report` the same report first.
    pub(crate) fn asked(
        &self,
        folder: &Path,
        report: &Report<'_>,
    ) -> Result<Option<Asked>, ReportError> {
        let input: Value = serde_json::from_slice(report.payload)
            .map_err(|why| ReportError::Unreadable(why.to_string()))?;
        let asked = |session: &str, asks| {
            Some(Asked {
                session: CliSession(session.to_owned()),
                terminal: report.terminal.map(str::to_owned),
                folder: text(&input, "cwd").map(PathBuf::from),
                asks,
            })
        };

        if input.get("tool_server").is_some() {
            // A call that names no session is asked for all the same: the Keeper then finds
            // the session by the process that called.
            let session = text(&input, "session_id").unwrap_or_default();
            return Ok(asked(session, Asks::Reply(reply_of(&input)?)));
        }

        let hook = text(&input, "hook_event_name")
            .ok_or_else(|| ReportError::Unreadable("it names no hook_event_name".to_owned()))?;
        let session = text(&input, "session_id")
            .ok_or_else(|| ReportError::Unreadable("it names no session_id".to_owned()))?;
        let asks = match hook {
            "UserPromptSubmit" => {
                let prompt = text(&input, "prompt").unwrap_or_default();
                (!prompt.trim_start().starts_with(SELF_STARTED)).then_some(Asks::Message)
            }
            "PreToolUse" => match text(&input, "tool_name") {
                Some("Read") => {
                    let file = text(&input["tool_input"], "file_path");
                    file.is_some_and(|file| is_own_review(folder, session, Path::new(file)))
                        .then_some(Asks::ReadOwnFile)
                }
                Some(LISTED_REPLY_TOOL) => Some(Asks::OwnTool),
                _ => None,
            },
            _ => None,
        };
        Ok(asks.and_then(|asks| asked(session, asks)))
    }

    /// What the piece hands Claude Code for `answer`, as this module's doc shows it: the
    /// allow of a `PreToolUse`, or the result of a call of the reply tool. An answer that
    /// does not belong to what `asked` waits for gives no bytes, which a hook prints as
    /// nothing: an allow never goes to a message.
    pub(crate) fn answer(&self, asked: &Asked, answer: &Answer<'_>) -> Vec<u8> {
        let answered = match (&asked.asks, answer) {
            (Asks::ReadOwnFile, Answer::Allow) => allow("a file of the review the app delivered"),
            (Asks::OwnTool, Answer::Allow) => allow("the app's own tool, which the user installed"),
            (
                Asks::Reply(_),
                Answer::Replied {
                    thread,
                    addressed,
                    characters,
                },
            ) => result(
                false,
                &format!(
                    "recorded reply to {thread} ({}, {characters} characters)",
                    if *addressed { "addressed" } else { "open" }
                ),
            ),
            (Asks::Reply(_), Answer::Refused { why }) => result(true, why),
            _ => return Vec::new(),
        };
        serde_json::to_vec(&answered).expect("JSON is written")
    }
}

/// The reply a call of the tool server holds: `thread` and `body` are text, `addressed` a
/// boolean that is false when left out.
fn reply_of(input: &Value) -> Result<AgentReply, ReportError> {
    let unreadable = |why: &str| ReportError::Unreadable(format!("the tool call {why}"));
    if text(input, "name") != Some(REPLY_TOOL) {
        return Err(unreadable("names a tool the server does not list"));
    }
    let arguments = &input["arguments"];
    let thread = text(arguments, "thread").ok_or_else(|| unreadable("names no thread"))?;
    let body = text(arguments, "body").ok_or_else(|| unreadable("holds no body"))?;
    let addressed = match arguments.get("addressed") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(addressed)) => *addressed,
        Some(_) => return Err(unreadable("holds an `addressed` that is no boolean")),
    };
    Ok(AgentReply {
        thread: thread.to_owned(),
        text: body.to_owned(),
        addressed,
    })
}

/// Whether `file`, made canonical, lies inside the folder of reviews of `session` under
/// `folder`, made canonical too. A folder that is not there yet holds nothing, and a path
/// that cannot be made canonical (there is no such file) is not allowed.
fn is_own_review(folder: &Path, session: &str, file: &Path) -> bool {
    if !is_folder_name(session) || !file.is_absolute() {
        return false;
    }
    let Ok(own) = folder.join(REVIEWS).join(session).canonicalize() else {
        return false;
    };
    file.canonicalize()
        .is_ok_and(|file| file != own && file.starts_with(&own))
}

/// The output of a `PreToolUse` hook that allows the call.
fn allow(why: &str) -> Value {
    json!({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "allow",
        "permissionDecisionReason": why,
    }})
}

/// The result of a call of the reply tool, as MCP gives it to Claude Code.
fn result(is_error: bool, message: &str) -> Value {
    let mut result = json!({"content": [{"type": "text", "text": message}]});
    if is_error {
        result["isError"] = json!(true);
    }
    result
}

/// The string `key` holds, if it holds one.
fn text<'a>(object: &'a Value, key: &str) -> Option<&'a str> {
    object.get(key)?.as_str()
}

/// Whether `name` can be one folder or file name of the app's own and no more: ASCII
/// letters, digits, `-` and `_`, as Claude Code's session ids are.
fn is_folder_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The delivery of one review that rides with a message of the session `session`: inline,
/// or as a file of the session's folder of reviews under `folder`, as this module's doc
/// says.
pub(crate) struct ToClaudeCode<'a> {
    /// The Adapter's own folder of the app.
    pub(crate) folder: &'a Path,
    /// The CLI session the message is of.
    pub(crate) session: &'a CliSession,
}

impl Delivery for ToClaudeCode<'_> {
    fn instruction(&self) -> &'static str {
        INSTRUCTION
    }

    /// The answer of the `UserPromptSubmit` hook that carries `rendered`; the file of a
    /// long review is written before the answer is given.
    fn hand_out(&mut self, rendered: &Rendered<'_>) -> io::Result<Vec<u8>> {
        // Checked for every delivery, so that a session whose id is no folder name gets
        // nothing, however short its review: the allow of its reads could name no folder.
        if !is_folder_name(&self.session.0) {
            return Err(invalid(format!(
                "the session id {:?} is not made of ASCII letters, digits, `-` and `_`",
                self.session.0
            )));
        }
        let context = if rendered.text.encode_utf16().count() <= INLINE_LIMIT {
            rendered.text.to_owned()
        } else {
            let file = self.write(rendered)?;
            format!(
                "{INSTRUCTION}\n\nThe review is too long to show here. It is the whole of this \
                 file:\n{}\nRead the whole file with your Read tool before you reply to the \
                 threads.",
                file.display()
            )
        };
        let line = format!(
            "yaar: review round {} attached ({} threads)",
            rendered.round, rendered.threads
        );
        let answer = json!({
            "hookSpecificOutput": {
                "hookEventName": "UserPromptSubmit",
                "additionalContext": context,
            },
            "systemMessage": line,
        });
        serde_json::to_vec(&answer).map_err(io::Error::other)
    }
}

impl ToClaudeCode<'_> {
    /// Writes `rendered` whole as the file of its round in the session's folder, through a
    /// temporary file renamed into place so that the agent never reads half of it, and
    /// gives the file's absolute path.
    fn write(&self, rendered: &Rendered<'_>) -> io::Result<PathBuf> {
        if !is_folder_name(rendered.review) {
            return Err(invalid(format!(
                "the review id {:?} is no file name",
                rendered.review
            )));
        }
        let folder = std::path::absolute(self.folder)?
            .join(REVIEWS)
            .join(&self.session.0);
        std::fs::create_dir_all(&folder)?;
        let name = format!("{}-{}.md", rendered.review, rendered.round);
        let file = folder.join(&name);
        let temporary = folder.join(format!(".{name}.{}.tmp", std::process::id()));
        let written = std::fs::write(&temporary, rendered.text)
            .and_then(|()| std::fs::rename(&temporary, &file));
        if written.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        written.map(|()| file)
    }
}

fn invalid(why: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, why)
}

/// The command that brings `session` back in the user's shell: `claude --resume` and the
/// id, single-quoted for `sh` (a `'` inside written `'\''`).
pub(crate) fn resume(session: &CliSession) -> String {
    format!("{COMMAND} --resume '{}'", session.0.replace('\'', r"'\''"))
}

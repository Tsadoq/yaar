//! The delivery module of Claude Code: how a review rides with the user's message, what
//! the hooks answer, and what the reply tool's server is answered with.
//!
//! A review rides in the answer of the `UserPromptSubmit` hook: inline, as
//! `hookSpecificOutput.additionalContext`, while the text is at most [`INLINE_LIMIT`]
//! characters, Claude Code's limit for one hook's output; else written whole to a file of
//! [`REVIEWS`] in the Adapter's own folder, outside every repository, and the answer holds
//! a short text that names that file and says to read it whole. Either answer also holds a
//! `systemMessage`, the one line the user sees under the prompt, as
//! `yaar: review round 2 attached (5 threads)`.
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
//! The second is given only for a `Read` of a file the Adapter wrote for that session and
//! for a call of [`LISTED_REPLY_TOOL`]: the two things the user allowed by installing the
//! app. A `PermissionRequest`, and every other hook, is answered with nothing.
//!
//! A call of the reply tool reaches the Adapter as a report of the tool server, whose
//! payload is a JSON object the server writes, `{"tool_server": "yaar", "session_id":
//! "<CLAUDE_CODE_SESSION_ID>", "cwd": "<its folder>", "name": "reply_to_thread",
//! "arguments": {...}}`; its answer is the `result` of the MCP call, as the server hands it
//! to Claude Code: `{"content": [{"type": "text", "text": "<what became of it>"}]}`, with
//! `"isError": true` when the reply was refused.

use std::io;
use std::path::Path;

use yaar_agent::{Answer, Asked, CliSession, Report, ReportError};

use crate::report::Remembered;

/// The longest text that rides inline in a hook's answer, in UTF-16 code units, as Claude
/// Code counts a string: 10,000 arrive whole, 10,001 are replaced by a path and a preview.
pub const INLINE_LIMIT: usize = 10_000;

/// The folder, inside the Adapter's own folder of the app, that holds the reviews too
/// long to ride inline, one file each: `<review id>-<round>.md`.
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

impl Remembered {
    /// What a report waits for: see [`yaar_agent::Adapter::asked`]. A `UserPromptSubmit`
    /// whose prompt opens, after white space, with `<task-notification>` is a message
    /// Claude Code started itself and asks nothing; any other prompt, whatever its words,
    /// is [`yaar_agent::Asks::Message`]. A `PreToolUse` of `Read` of a file this Adapter
    /// wrote for that session is [`yaar_agent::Asks::ReadOwnFile`], one of
    /// [`LISTED_REPLY_TOOL`] is [`yaar_agent::Asks::OwnTool`]. A report of the tool server
    /// is [`yaar_agent::Asks::Reply`].
    pub(crate) fn asked(&self, report: &Report<'_>) -> Result<Option<Asked>, ReportError> {
        let _ = report;
        todo!()
    }

    /// What the piece hands Claude Code for `answer`, as this module's doc shows it. A
    /// review too long to ride inline is written into `folder`/[`REVIEWS`] through a
    /// temporary file renamed into place, and remembered as a file of `asked`'s session.
    pub(crate) fn answer(
        &mut self,
        folder: &Path,
        asked: &Asked,
        answer: &Answer<'_>,
    ) -> io::Result<Vec<u8>> {
        let _ = (folder, asked, answer);
        todo!()
    }
}

/// The command that brings `session` back in the user's shell: `claude --resume` and the
/// id, single-quoted for `sh` (a `'` inside written `'\''`).
pub(crate) fn resume(session: &CliSession) -> String {
    let _ = session;
    todo!()
}

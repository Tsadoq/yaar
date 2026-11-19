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
use std::path::Path;

use yaar_agent::{Answer, Asked, CliSession, Delivery, Rendered, Report, ReportError};

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

impl Remembered {
    /// What a report waits for: see [`yaar_agent::Adapter::asked`]. A `UserPromptSubmit`
    /// whose prompt opens, after white space, with `<task-notification>` is a message
    /// Claude Code started itself and asks nothing; any other prompt, whatever its words,
    /// is [`yaar_agent::Asks::Message`]. A `PreToolUse` of `Read` of a file inside the
    /// folder of reviews of that session under `folder` (this module's doc) is
    /// [`yaar_agent::Asks::ReadOwnFile`], one of [`LISTED_REPLY_TOOL`] is
    /// [`yaar_agent::Asks::OwnTool`]. A report of the tool server is
    /// [`yaar_agent::Asks::Reply`]. Gives none until the package that builds delivery fills
    /// it in, so that every report of slices 4 and 5 is followed as before.
    pub(crate) fn asked(
        &self,
        folder: &Path,
        report: &Report<'_>,
    ) -> Result<Option<Asked>, ReportError> {
        let _ = (folder, report);
        Ok(None)
    }

    /// What the piece hands Claude Code for `answer`, as this module's doc shows it: the
    /// allow of a `PreToolUse`, or the result of a call of the reply tool.
    pub(crate) fn answer(&self, asked: &Asked, answer: &Answer<'_>) -> Vec<u8> {
        let _ = (asked, answer);
        todo!()
    }
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
        let _ = (rendered, self.folder, self.session);
        todo!()
    }
}

/// The command that brings `session` back in the user's shell: `claude --resume` and the
/// id, single-quoted for `sh` (a `'` inside written `'\''`).
pub(crate) fn resume(session: &CliSession) -> String {
    let _ = session;
    todo!()
}

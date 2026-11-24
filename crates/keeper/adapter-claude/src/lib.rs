//! Claude Code adapter: maps Claude Code's hook events to the seven events, delivers reviews
//! and replies, writes the plugin.
//!
//! The hook command is `exec '<hook>'`: Claude Code runs it through `sh -c`, and `exec`
//! replaces that `sh` (dash would keep a lone command in a child), so the hook program's
//! parent is Claude Code itself, whose process the Keeper watches.
//!
//! Every event carries the hook input's `cwd` as its session's folder, and its `session_id`.
//!
//! | Hook | Event |
//! | --- | --- |
//! | `SessionStart`, `source` `startup` or `clear` | session started, new |
//! | `SessionStart`, `source` `resume` | session started, resumed |
//! | `UserPromptSubmit` | turn started |
//! | `PreToolUse` of `Bash` | command began |
//! | `PermissionRequest` | waiting for permission on |
//! | `PostToolUse` after a `PermissionRequest` of the session | waiting off, then what follows |
//! | `PostToolUse` of `Read` | file read, `tool_input.file_path` |
//! | `PostToolUse` of `Edit`, `MultiEdit`, `NotebookEdit` | file written, with lines (below) |
//! | `PostToolUse` of `Write` | file written by the write tool, no lines |
//! | `PostToolUse` or `PostToolUseFailure` of `Bash` | command ended |
//! | `Stop` | turn ended, finished |
//! | `SessionEnd` | session ended |
//! | an interrupt mark while a turn is open | waiting off if it waited, turn ended, interrupted |
//!
//! Edit lines: for each hunk of `tool_response.structuredPatch`, `newStart` to
//! `newStart + newLines - 1`, or `newStart` alone when `newLines` is 0; none without a patch.
//!
//! Any other hook gives nothing; a tool event inside a sub-agent is the session's own. A dialog
//! denied and an Esc run no hook: Claude Code writes the mark into its terminal. A shell
//! command denied or interrupted is never reported ended: the Switchboard ends it with its turn.
//!
//! Claude Code writes the mark some milliseconds after `Stop`, once its `Stop` hooks have
//! returned. The Keeper queues a report before it answers the hook, so a `Stop` is taken
//! before the mark that follows it, which then means nothing.

mod deliver;
mod files;
mod install;
mod report;

pub use deliver::{
    INLINE_LIMIT, INSTRUCTION, LISTED_REPLY_TOOL, REPLY_TOOL, REVIEWS, TOOL_SERVER,
    TOOL_SERVER_NAME,
};

use std::io;
use std::path::Path;
use std::time::SystemTime;

use report::Remembered;
use yaar_agent::{
    Adapter, Answer, Asked, CliSession, Delivery, Report, ReportError, Reported, ShellEntry,
};

pub const NAME: &str = "claude";

pub const TITLE: &str = "Claude Code";

pub const COMMAND: &str = "claude";

/// The variable that names the plugin folders Claude Code loads, a list parted by `:`.
pub const PLUGIN_DIRS: &str = "CLAUDE_CODE_PLUGIN_DIRS";

/// Claude Code names the app's tools `mcp__plugin_yaar-review_<server>__<tool>`.
pub const PLUGIN: &str = "yaar-review";

/// The program Claude Code runs for each hook; it lies in the folder of the app's programs,
/// beside the Keeper.
pub const HOOK_PROGRAM: &str = "yaar-claude-hook";

/// `PostToolUseFailure` is among them because a failing shell command ends there, not in
/// `PostToolUse`.
pub const HOOK_EVENTS: [&str; 8] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "PostToolUseFailure",
    "Stop",
    "SessionEnd",
];

/// What Claude Code writes into its terminal when a turn stops, also by an interrupt or a
/// denied permission: the progress sequence for "no progress", ended by BEL or by the string
/// terminator, with or without the last `;`.
pub const INTERRUPT_MARKS: [&[u8]; 4] = [
    b"\x1b]9;4;0;\x07",
    b"\x1b]9;4;0\x07",
    b"\x1b]9;4;0;\x1b\\",
    b"\x1b]9;4;0\x1b\\",
];

#[derive(Debug, Default)]
pub struct ClaudeCode {
    memory: Remembered,
}

impl ClaudeCode {
    pub fn new() -> ClaudeCode {
        ClaudeCode::default()
    }
}

impl Adapter for ClaudeCode {
    fn name(&self) -> &'static str {
        NAME
    }

    fn title(&self) -> &'static str {
        TITLE
    }

    fn command(&self) -> &'static str {
        COMMAND
    }

    /// Writes `folder/plugin`: `.claude-plugin/plugin.json`, `hooks/hooks.json` (one group per
    /// event, no `matcher`: every tool) and `.mcp.json`, each through a temporary file renamed
    /// into place. Gives [`PLUGIN_DIRS`] with `folder/plugin`.
    fn install(&self, folder: &Path, companions: &Path) -> io::Result<Vec<ShellEntry>> {
        install::write(folder, companions)
    }

    fn marks(&self) -> Vec<Vec<u8>> {
        INTERRUPT_MARKS.iter().map(|mark| mark.to_vec()).collect()
    }

    fn report(&mut self, report: &Report<'_>) -> Result<Vec<Reported>, ReportError> {
        self.memory.report(report)
    }

    fn marked(&mut self, terminal: &str, mark: usize) -> Vec<Reported> {
        if mark >= INTERRUPT_MARKS.len() {
            return Vec::new();
        }
        self.memory.interrupted(terminal)
    }

    fn asked(&self, folder: &Path, report: &Report<'_>) -> Result<Option<Asked>, ReportError> {
        self.memory.asked(folder, report)
    }

    fn instruction(&self) -> &'static str {
        INSTRUCTION
    }

    fn delivery<'a>(&'a self, folder: &'a Path, asked: &'a Asked) -> Box<dyn Delivery + 'a> {
        Box::new(deliver::ToClaudeCode {
            folder,
            session: &asked.session,
        })
    }

    fn answer(&self, asked: &Asked, answer: &Answer<'_>) -> Vec<u8> {
        self.memory.answer(asked, answer)
    }

    fn resume(&self, session: &CliSession, cli_record: Option<&Path>) -> String {
        // Claude Code resumes a session by its id alone.
        let _ = cli_record;
        deliver::resume(session)
    }

    fn remove_files_of_ended(
        &self,
        folder: &Path,
        runs: &dyn Fn(&str) -> bool,
        written_before: Option<SystemTime>,
    ) -> io::Result<u64> {
        files::remove_files_of_ended(folder, runs, written_before)
    }
}

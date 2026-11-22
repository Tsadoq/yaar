//! Claude Code adapter (`H-AGENT-CC`).
//!
//! Owns: the mapping of Claude Code's hook events to the seven events; its delivery, reply
//! handling, shell entries, launcher and resume command.
//!
//! Surface: [`ClaudeCode`], the [`yaar_agent::Adapter`] of Claude Code, and the names it
//! stands on: [`NAME`], [`TITLE`], [`COMMAND`], [`PLUGIN_DIRS`], [`PLUGIN`],
//! [`HOOK_PROGRAM`], [`HOOK_EVENTS`] and [`INTERRUPT_MARKS`]; and those of its delivery
//! module (`deliver.rs`): [`INSTRUCTION`], [`INLINE_LIMIT`], [`REVIEWS`], [`TOOL_SERVER`],
//! [`TOOL_SERVER_NAME`], [`REPLY_TOOL`] and [`LISTED_REPLY_TOOL`].
//!
//! How Claude Code reaches the app. The Adapter writes a plugin folder into its own folder
//! of the app: `plugin/.claude-plugin/plugin.json`, which names the plugin [`PLUGIN`], and
//! `plugin/hooks/hooks.json`, which runs the hook program [`HOOK_PROGRAM`] of the app for
//! each event of [`HOOK_EVENTS`]. Every shell of the app gets [`PLUGIN_DIRS`] with that
//! folder added, and a plain `claude` typed there, or started from its launcher, loads the
//! plugin. Nothing is written into the user's settings or into a worktree.
//!
//! The two files, with `<hook>` the absolute path of the hook program quoted for `sh`, and
//! one entry under `hooks` for each event of [`HOOK_EVENTS`], each a list of one group
//! with no `matcher`, which Claude Code reads as every tool:
//!
//! ```json
//! {"name": "yaar-review", "version": "0.1.0", "description": "..."}
//! ```
//!
//! ```json
//! {"hooks": {
//!   "SessionStart": [{"hooks": [{"type": "command", "command": "exec '<hook>'"}]}],
//!   "PreToolUse": [{"hooks": [{"type": "command", "command": "exec '<hook>'"}]}]
//! }}
//! ```
//!
//! A group may also carry `"matcher": "Edit|Write"`, the tools it is for, parted by `|`;
//! the app's own groups carry none, and the stand-in agent of the system tests reads both.
//!
//! The plugin also serves the reply tool: `plugin/.mcp.json` names the tool server
//! [`TOOL_SERVER_NAME`], which Claude Code starts as the absolute path of the program
//! [`TOOL_SERVER`] beside the hook program, with no argument:
//!
//! ```json
//! {"mcpServers": {"yaar": {"command": "<tools>", "args": []}}}
//! ```
//!
//! Claude Code lists its one tool to the agent as [`LISTED_REPLY_TOOL`]. What the hooks
//! answer, and how a review rides with a message, is the delivery module's: see
//! `deliver.rs`.
//!
//! The command is `exec` and the quoted path and nothing else, no redirection and no `;`:
//! Claude Code runs it through `sh -c`, and `exec` puts the hook program in the place of
//! that `sh` (dash, the `sh` of Ubuntu, would keep a lone command in a child otherwise), so
//! that the hook program's parent is Claude Code itself, whose process the Keeper watches.
//! A command of several parts would leave a short-lived `sh` as that parent.
//!
//! Every event carries the hook input's `cwd` as the folder of its session, and its
//! `session_id`. Paths are given as the CLI named them, absolute.
//!
//! What the hooks report, read as the app's events:
//!
//! | Hook | Event |
//! | --- | --- |
//! | `SessionStart`, `source` `startup` or `clear` | session started, new |
//! | `SessionStart`, `source` `resume` | session started, resumed |
//! | `UserPromptSubmit` | turn started |
//! | `PreToolUse` of `Bash` | command began |
//! | `PermissionRequest` | waiting for permission on |
//! | `PostToolUse` after a `PermissionRequest` of the session | waiting for permission off, then what follows |
//! | `PostToolUse` of `Read` | file read, `tool_input.file_path` |
//! | `PostToolUse` of `Edit`, `MultiEdit`, `NotebookEdit` | file written by the edit tool, with the lines of `tool_response.structuredPatch`: for each hunk `newStart` to `newStart + newLines - 1`, or `newStart` alone when `newLines` is 0; none without a patch |
//! | `PostToolUse` of `Write` | file written by the write tool, no lines |
//! | `PostToolUse` or `PostToolUseFailure` of `Bash` | command ended |
//! | `Stop` | turn ended, finished |
//! | `SessionEnd` | session ended |
//! | the mark [`INTERRUPT_MARKS`] in the session's terminal while a turn is open | waiting off if it waited, turn ended, interrupted |
//!
//! Any other hook, as `InstructionsLoaded`, `PostToolBatch`, `Notification`,
//! `SubagentStart` or `SubagentStop`, gives nothing; a tool event inside a sub-agent is
//! the session's own. A dialog denied, and an Esc during a turn, run no hook at all:
//! Claude Code writes the mark into its terminal then, and only the mark tells. A shell
//! command that began and was denied or interrupted is never reported ended here: the
//! Switchboard ends it with its turn.
//!
//! The mark follows every turn's end, `Stop` included, by some milliseconds, and Claude
//! Code writes it only once its `Stop` hooks have returned: the Keeper queues a report
//! before it answers the hook, and a mark on the same queue, so a `Stop` is always taken
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

/// The name the app knows Claude Code by.
pub const NAME: &str = "claude";

/// The name a person reads.
pub const TITLE: &str = "Claude Code";

/// What a launcher runs in the user's shell.
pub const COMMAND: &str = "claude";

/// The variable that names the plugin folders Claude Code loads, a list parted by `:`.
pub const PLUGIN_DIRS: &str = "CLAUDE_CODE_PLUGIN_DIRS";

/// The name of the app's plugin, as `plugin.json` gives it: the app's name where a name
/// must be unique, since it must be unique among the user's plugins. Claude Code names the
/// app's tools after it, `mcp__plugin_yaar-review_<server>__<tool>`.
pub const PLUGIN: &str = "yaar-review";

/// The program of the app that Claude Code runs for each hook: it lies in the folder of
/// the app's programs, beside the Keeper.
pub const HOOK_PROGRAM: &str = "yaar-claude-hook";

/// The hooks the plugin asks for, each running [`HOOK_PROGRAM`]. `PostToolUseFailure` is
/// among them because a shell command that fails ends there, not in `PostToolUse`.
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

/// What Claude Code writes into its terminal when a turn stops, also one the user
/// interrupted or a permission the user denied: the progress sequence that says "no
/// progress", ended by BEL or by the string terminator, with or without the last `;`.
pub const INTERRUPT_MARKS: [&[u8]; 4] = [
    b"\x1b]9;4;0;\x07",
    b"\x1b]9;4;0\x07",
    b"\x1b]9;4;0;\x1b\\",
    b"\x1b]9;4;0\x1b\\",
];

/// The Adapter of Claude Code. It remembers, for each terminal, the session that last
/// reported from it, whether a turn of it is open and whether it waits for a permission.
#[derive(Debug, Default)]
pub struct ClaudeCode {
    memory: Remembered,
}

impl ClaudeCode {
    /// The Adapter, knowing no session yet.
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

    /// Writes `folder/plugin`: `.claude-plugin/plugin.json`, `hooks/hooks.json` and
    /// `.mcp.json` as the crate's doc shows them, whose every hook of [`HOOK_EVENTS`] runs
    /// `companions/`[`HOOK_PROGRAM`] alone after `exec`, single-quoted for the shell Claude
    /// Code runs it with (a `'` inside written `'\''`), and whose tool server is
    /// `companions/`[`TOOL_SERVER`] by its absolute path, which Claude Code starts with no
    /// shell. Each file is written to a temporary file of its folder and renamed into place.
    /// Gives [`PLUGIN_DIRS`] with `folder/plugin`.
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
        // Every form of the mark says the same.
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

    /// Claude Code brings a session back by its id alone: `cli_record` is not read.
    fn resume(&self, session: &CliSession, cli_record: Option<&Path>) -> String {
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

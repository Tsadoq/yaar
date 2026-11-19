//! Claude Code adapter (`H-AGENT-CC`).
//!
//! Owns: the mapping of Claude Code's hook events to the seven events; its delivery, reply
//! handling, shell entries, launcher and resume command.
//!
//! Surface: [`ClaudeCode`], the [`yaar_agent::Adapter`] of Claude Code, and the names it
//! stands on: [`NAME`], [`TITLE`], [`COMMAND`], [`PLUGIN_DIRS`], [`PLUGIN`],
//! [`HOOK_PROGRAM`], [`HOOK_EVENTS`] and [`INTERRUPT_MARKS`].
//!
//! How Claude Code reaches the app. The Adapter writes a plugin folder into its own folder
//! of the app: `plugin/.claude-plugin/plugin.json`, which names the plugin [`PLUGIN`], and
//! `plugin/hooks/hooks.json`, which runs the hook program [`HOOK_PROGRAM`] of the app for
//! each event of [`HOOK_EVENTS`]. Every shell of the app gets [`PLUGIN_DIRS`] with that
//! folder added, and a plain `claude` typed there, or started from its launcher, loads the
//! plugin. Nothing is written into the user's settings or into a worktree.
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
//! | `PostToolUse` of `Edit`, `MultiEdit`, `NotebookEdit` | file written by the edit tool |
//! | `PostToolUse` of `Write` | file written by the write tool |
//! | `PostToolUse` or `PostToolUseFailure` of `Bash` | command ended |
//! | `Stop` | turn ended, finished |
//! | `SessionEnd` | session ended |
//! | the mark [`INTERRUPT_MARKS`] in the session's terminal while a turn is open | waiting off if it waited, turn ended, interrupted |
//!
//! Any other hook, as `InstructionsLoaded`, `PostToolBatch`, `Notification`,
//! `SubagentStart` or `SubagentStop`, gives nothing; a tool event inside a sub-agent is
//! the session's own. A dialog denied, and an Esc during a turn, run no hook at all:
//! Claude Code writes the mark into its terminal then, and only the mark tells.

use std::io;
use std::path::Path;

use yaar_agent::{Adapter, Report, ReportError, Reported, ShellEntry};

/// The name the app knows Claude Code by.
pub const NAME: &str = "claude";

/// The name a person reads.
pub const TITLE: &str = "Claude Code";

/// What a launcher runs in the user's shell.
pub const COMMAND: &str = "claude";

/// The variable that names the plugin folders Claude Code loads, a list parted by `:`.
pub const PLUGIN_DIRS: &str = "CLAUDE_CODE_PLUGIN_DIRS";

/// The name of the app's plugin, as `plugin.json` gives it; Claude Code names the app's
/// tools after it.
pub const PLUGIN: &str = "yaar";

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
    _todo: (),
}

impl ClaudeCode {
    /// The Adapter, knowing no session yet.
    pub fn new() -> ClaudeCode {
        todo!("an adapter that knows no session")
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

    /// Writes `folder/plugin`: `.claude-plugin/plugin.json` and `hooks/hooks.json`, whose
    /// every hook of [`HOOK_EVENTS`] runs `companions/`[`HOOK_PROGRAM`], quoted for the
    /// shell Claude Code runs it with. Gives [`PLUGIN_DIRS`] with `folder/plugin`.
    fn install(&self, folder: &Path, companions: &Path) -> io::Result<Vec<ShellEntry>> {
        let _ = (folder, companions);
        todo!("the plugin folder and the entry that loads it")
    }

    fn marks(&self) -> Vec<Vec<u8>> {
        todo!("the interrupt marks")
    }

    fn report(&mut self, report: &Report<'_>) -> Result<Vec<Reported>, ReportError> {
        let _ = report;
        todo!("the events of one hook's input")
    }

    fn marked(&mut self, terminal: &str, mark: usize) -> Vec<Reported> {
        let _ = (terminal, mark);
        todo!("the events of an interrupt mark")
    }
}

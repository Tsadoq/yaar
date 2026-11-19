//! The stand-in agent CLI of the system tests: the program `standin-agent`, and the script
//! a test hands it.
//!
//! It stands in for the agent CLI itself, and for nothing of the app. A test puts it on
//! the search path of its world under the names `claude`, `pi` and `otheragent`, and starts
//! it as a user starts a CLI: from a launcher of the app, or by its name typed in a shell
//! of the app. It runs in the app's terminal, reads the keys typed there, and writes there
//! what such a CLI writes.
//!
//! Started as `claude` it behaves as Claude Code does towards the app's pieces: it reads
//! the plugin folders `CLAUDE_CODE_PLUGIN_DIRS` names, and for each event of its script
//! runs every command `hooks/hooks.json` of those folders gives for that event, through
//! `/bin/sh -c`, with the event's input on its standard input, as the shapes recorded from
//! the real Claude Code have it, and waits for it. That command is the app's own hook
//! program, which reports to the Keeper over its socket: the stand-in knows neither the
//! Keeper, nor the Link, nor the socket. As `otheragent` it runs no hook at all: a CLI the
//! app does not support, that only edits files. As `pi` it refuses to start in slice 4,
//! saying the pi dialect comes in slice 7. With [`IGNORE_PIECES`] set it runs no hook
//! either, as a CLI started around the app's pieces.
//!
//! Its script is a file, named by [`SCRIPT_VARIABLE`], of [`Step`]s, one to a line. It
//! decides nothing: it follows the script and the keys. What reached it, every byte read
//! from its terminal, every hook's answer and its own arguments, it writes down in the
//! file [`LOG_VARIABLE`] names: the agent's side of a test, read with [`StandinLog`].

use std::path::{Path, PathBuf};

/// The variable that names the stand-in's script.
pub const SCRIPT_VARIABLE: &str = "STANDIN_SCRIPT";

/// The variable that names the file the stand-in writes down what reached it in.
pub const LOG_VARIABLE: &str = "STANDIN_LOG";

/// The variable that, set to `1`, makes the stand-in run none of the app's pieces.
pub const IGNORE_PIECES: &str = "STANDIN_IGNORE_PIECES";

/// What the stand-in prints when it is ready for the user's next message, at the start
/// of a line: a test that types a message waits for it first, and looks for the stand-in's
/// own words, never for a shell's prompt.
pub const PROMPT: &str = "standin> ";

/// One step of a stand-in's script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Prints [`PROMPT`] and waits for a line typed at it, ended by Enter: the user's
    /// message. Its submission starts a turn: `UserPromptSubmit`. A line typed while a
    /// turn runs is held until the turn ends, and then starts the next, as both CLIs do.
    Turn,
    /// Reads a file of the worktree with its read tool: `PreToolUse` and `PostToolUse` of
    /// `Read`, the file really read in between.
    Read(PathBuf),
    /// Adds a line to a file with its edit tool: `PreToolUse` and `PostToolUse` of
    /// `Edit`, the file really changed in between.
    Edit(PathBuf, String),
    /// Writes a file whole with its write tool, a new file among them: `PreToolUse` and
    /// `PostToolUse` of `Write`.
    Write(PathBuf, String),
    /// Runs a shell command in the worktree, through `/bin/sh -c`: `PreToolUse` of
    /// `Bash`, the command, `PostToolUse` of `Bash`.
    Shell(String),
    /// Asks for permission to write a file whole: `PreToolUse` and `PermissionRequest` of
    /// `Write`, then a dialog printed in the terminal that waits for one key. `y` writes
    /// the file and gives `PostToolUse`; `n` or Esc denies it, prints the mark Claude Code
    /// prints, [`INTERRUPT_MARK`], gives no event, and ends the turn as interrupted.
    Ask(PathBuf, String),
    /// Holds the turn open until the file exists: a test that wants to act while the
    /// agent works makes the file when it is done.
    Hold(PathBuf),
    /// Writes this many numbered lines into its terminal as fast as it can, and writes
    /// down in its log the longest a single write waited.
    Flood(u32),
    /// Ends the turn: `Stop`. Esc pressed during a turn, before this step, prints
    /// [`INTERRUPT_MARK`], gives no event, and ends the turn as interrupted.
    End,
    /// Ends the session, `SessionEnd`, and the program.
    Exit,
}

/// The mark Claude Code prints when a turn stops: the progress sequence that says no
/// progress, as the recordings hold it, then the words it prints on an interrupt.
pub const INTERRUPT_MARK: &[u8] =
    b"\x1b]9;4;0;\x07Interrupted \xc2\xb7 What should Claude do instead?\r\n";

/// A stand-in's script.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Script(pub Vec<Step>);

impl Script {
    /// The script as the file the stand-in reads: one step to a line, a keyword and its
    /// words, a text to the end of the line with `\n` for a line end inside it.
    pub fn text(&self) -> String {
        todo!("the script as text")
    }

    /// Reads a script from the text [`Script::text`] writes. Panics, naming the line, on
    /// one it cannot read.
    pub fn parse(text: &str) -> Script {
        let _ = text;
        todo!("a script from its text")
    }
}

/// What reached a stand-in, as it wrote it down.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StandinLog {
    /// Its process id.
    pub pid: u32,
    /// Its arguments, without its own name.
    pub args: Vec<String>,
    /// The CLI's own session id it gave itself, or the one it was told to resume.
    pub session: String,
    /// Every byte it read from its terminal, in order.
    pub received: Vec<u8>,
    /// The bytes it had read when each step began, by the step's index: a test reads what
    /// reached it between two steps from the difference.
    pub received_at_step: Vec<usize>,
    /// What each hook it ran printed, in order, with the hook's event.
    pub answers: Vec<(String, Vec<u8>)>,
    /// The longest a single write of a [`Step::Flood`] waited, in microseconds.
    pub longest_write_us: Option<u64>,
}

impl StandinLog {
    /// Reads the log the stand-in wrote at `path`; an empty log when the file is not
    /// there yet.
    pub fn read(path: &Path) -> StandinLog {
        let _ = path;
        todo!("the log a stand-in wrote")
    }
}

/// The stand-in's program: what `standin-agent` runs, given its name as it was started,
/// its arguments, and its environment through the process. Gives its exit status.
pub fn run(name: &str, args: &[String]) -> i32 {
    let _ = (name, args);
    todo!("the stand-in agent")
}

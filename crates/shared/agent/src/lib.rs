//! Agent contract (`seam of NFR-001 and NFR-003`).
//!
//! Owns: the seven events an agent CLI is turned into, and what an Adapter and a delivery
//! module must supply. It owns no behaviour.
//!
//! Surface:
//! - [`Event`], the seven events: a session started or ended, a turn started or ended, a
//!   file read, a file written, waiting for the user's permission on or off; and
//!   [`Event::Command`], the step of a shell command, which names no file but tells when a
//!   command runs, so that a write the watcher sees can be credited to the session whose
//!   command was running;
//! - [`Reported`], an event with the CLI's own session and the terminal and process it
//!   came from; [`CliSession`], the CLI's own name for a session;
//! - [`Report`], what a piece of the app inside the CLI passed on, as the Keeper hands it
//!   to the Adapter;
//! - [`Adapter`], what one integration module of a CLI supplies: its names, the pieces it
//!   writes into the app's folder and the entries every shell of the app gets for them,
//!   the marks of the terminal output it watches for, and the reading of its reports;
//!   [`ShellEntry`] and [`ReportError`].
//!
//! Every event names its session, and a write made with the CLI's edit tool names its
//! path: a marker per agent in Flow, after the first release, needs both.
//!
//! What later slices add here, each beside what is here and changing none of it: a
//! delivery module (the review that rides with the user's next message, and how the
//! piece answers with it), the reading of the agent's replies to threads, and the command
//! that resumes a session.
//!
//! Nothing here knows the Link, the Store or the Window. The Keeper program lists the
//! Adapters, and no other crate of the app names one.

use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// The CLI's own name for one of its sessions, as its reports carry it: for Claude Code
/// the `session_id` of every hook's input. A session the CLI resumes keeps it; a session
/// the CLI starts anew, after a clear for example, has another.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CliSession(pub String);

impl fmt::Display for CliSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a file write was made with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WriteTool {
    /// The CLI's edit tool: part of an existing file was replaced.
    Edit,
    /// The CLI's write tool: the file was written whole, a new file among them.
    Write,
}

/// Whether a shell command begins or has ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    /// The CLI is about to run it.
    Began,
    /// It has run.
    Ended,
}

/// How a session started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Start {
    /// A new session.
    New,
    /// A session of the CLI that ran before, resumed: it keeps its [`CliSession`].
    Resumed,
}

/// How a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TurnEnd {
    /// The agent finished its answer.
    Finished,
    /// The user interrupted it, or denied a permission it waited for: the CLI reported no
    /// end, and its terminal showed the mark that says so.
    Interrupted,
}

/// What an agent CLI did, in the app's words. Every Adapter turns its CLI's reports into
/// these and nothing else; everything else in the app reads these.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Event {
    /// A session started.
    SessionStarted {
        /// New or resumed.
        how: Start,
    },
    /// The session ended, as the CLI reported it. A process that is killed reports
    /// nothing: the Keeper sees it gone.
    SessionEnded,
    /// The agent started a turn: the user submitted a message, or the CLI started one
    /// itself, as Claude Code does when a background task ends.
    TurnStarted,
    /// The turn ended.
    TurnEnded {
        /// How.
        how: TurnEnd,
    },
    /// The agent read a file with its own tool.
    FileRead {
        /// The path as the CLI named it.
        path: PathBuf,
    },
    /// The agent wrote a file with its own tool; told once the write is done.
    FileWritten {
        /// The path as the CLI named it.
        path: PathBuf,
        /// What it was written with.
        tool: WriteTool,
    },
    /// A shell command of the agent begins or has ended. It names no file: what it writes
    /// is seen by the watcher of the worktree.
    Command {
        /// Whether it begins or has ended.
        phase: Phase,
        /// The command, as the agent wrote it.
        command: String,
    },
    /// The agent waits for the user's answer at a permission prompt, or no longer does.
    Permission {
        /// Whether it waits.
        waiting: bool,
    },
}

/// One event, with where it came from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Reported {
    /// The CLI's own name for the session.
    pub session: CliSession,
    /// The terminal of the app the CLI runs in, by the id the app gave it; none outside
    /// the app's terminals.
    pub terminal: Option<String>,
    /// The process of the CLI.
    pub pid: u32,
    /// What happened.
    pub event: Event,
}

/// What a piece of the app inside a CLI passed on, as the Keeper hands it to the
/// Adapter of that CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report<'a> {
    /// The terminal of the app the CLI runs in, as the piece's environment named it.
    pub terminal: Option<&'a str>,
    /// The process of the CLI: the piece's parent.
    pub pid: u32,
    /// What the CLI handed the piece, byte for byte.
    pub payload: &'a [u8],
}

/// What every shell of the app gets for one CLI, so that the CLI, typed by name or
/// started from its launcher, loads the app's pieces. The Keeper puts these into the
/// environment of every shell it starts, beside the address of its socket and the id of
/// the terminal, and nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellEntry {
    /// A variable whose value is a list of paths parted by `:`: this one is added to the
    /// end of the value the user's own environment holds, or is the whole value where it
    /// holds none. Claude Code's `CLAUDE_CODE_PLUGIN_DIRS` is one.
    PathList {
        /// The variable.
        name: OsString,
        /// The path added to it.
        path: PathBuf,
    },
    /// A folder put first on the search path, `PATH`, as for pi's wrapper.
    SearchFirst(PathBuf),
}

/// Why a report could not be read. The Keeper answers the piece with it; the CLI goes on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportError {
    /// The payload is not what this CLI hands its pieces: what is wrong, as a sentence.
    Unreadable(String),
}

impl fmt::Display for ReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(why) => write!(f, "the report cannot be read: {why}"),
        }
    }
}

impl std::error::Error for ReportError {}

/// One integration module of an agent CLI: everything the app knows of that CLI. Adding
/// a CLI is adding one of these and the CLI's own piece, and changing nothing else.
///
/// An Adapter keeps what it must remember between reports of its CLI, as which session
/// runs in which terminal and whether a turn is open there; the Keeper calls it from one
/// thread at a time.
pub trait Adapter: Send {
    /// The name the Link, the Store and the other parts know the CLI by: `claude`.
    fn name(&self) -> &'static str;

    /// The name a person reads: `Claude Code`.
    fn title(&self) -> &'static str;

    /// What a launcher of the CLI runs in the user's shell, as the user would type it:
    /// `claude`.
    fn command(&self) -> &'static str;

    /// Writes the CLI's pieces into `folder`, a folder of the app's own that belongs to
    /// this Adapter, and gives the entries every shell of the app gets so that the CLI
    /// loads them. `companions` is the folder of the app's programs that pass a CLI's
    /// reports on, where the pieces find them. Called at every start of the Keeper; what
    /// was written before is written again. Nothing is written anywhere else: not into
    /// the user's settings of the CLI, not into a worktree.
    fn install(&self, folder: &Path, companions: &Path) -> io::Result<Vec<ShellEntry>>;

    /// The runs of bytes in a terminal's output that this CLI writes and the Keeper must
    /// hear of, as the end of a turn the user interrupted: the Dock watches every
    /// terminal of the app for them and tells [`Adapter::marked`] by their index.
    fn marks(&self) -> Vec<Vec<u8>>;

    /// Turns one report of the CLI's piece into events, in order. A report of something
    /// the app does not follow gives none.
    fn report(&mut self, report: &Report<'_>) -> Result<Vec<Reported>, ReportError>;

    /// The mark of index `mark` of [`Adapter::marks`] was seen in the output of
    /// `terminal`: the events it means, none when it means nothing now, as the mark that
    /// follows a turn already reported ended.
    fn marked(&mut self, terminal: &str, mark: usize) -> Vec<Reported>;
}

//! Agent CLI events, and the traits an Adapter and a delivery module implement.

use std::ffi::OsString;
use std::fmt;
use std::io;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

/// The CLI's own session id. Kept when the CLI resumes the session, new after a clear.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CliSession(pub String);

impl fmt::Display for CliSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WriteTool {
    Edit,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    Began,
    Ended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Start {
    New,
    Resumed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TurnEnd {
    Finished,
    /// The user interrupted, or denied a permission it waited for: the CLI reported no end.
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Event {
    SessionStarted {
        how: Start,
    },
    /// A killed process reports nothing: the Keeper sees it gone.
    SessionEnded,
    TurnStarted,
    TurnEnded {
        how: TurnEnd,
    },
    FileRead {
        path: PathBuf,
    },
    FileWritten {
        path: PathBuf,
        tool: WriteTool,
        /// The lines of the file as it is now, numbered from 1: for each part an edit replaced, the
        /// lines that took its place, or the line where a removed part was. None for a file written
        /// whole, and where the CLI did not say.
        lines: Option<Vec<RangeInclusive<u32>>>,
    },
    /// Names no file: what it writes is seen by the worktree watcher.
    Command {
        phase: Phase,
        command: String,
    },
    Permission {
        waiting: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Reported {
    pub session: CliSession,
    /// The app's id of the terminal the CLI runs in; None outside the app's terminals.
    pub terminal: Option<String>,
    pub pid: u32,
    pub folder: Option<PathBuf>,
    /// The CLI's own session file: for pi, what its resume command needs.
    pub cli_record: Option<PathBuf>,
    pub event: Event,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report<'a> {
    pub terminal: Option<&'a str>,
    /// The agent CLI's process as the piece found it: for Claude Code's hook program, its parent.
    pub pid: u32,
    pub payload: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellEntry {
    /// This path is appended to the user's value, or is the whole value where there is none.
    PathList {
        name: OsString,
        path: PathBuf,
    },
    Variable {
        name: OsString,
        value: OsString,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentAnswer {
    NotAsked,
    Yes,
    No,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Consent {
    pub question: String,
    pub answer: ConsentAnswer,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportError {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asks {
    /// The user submitted a message, queued ones among them: what is attached for the session
    /// rides with it. Never a message the CLI starts itself, nor the answer to any dialog.
    Message,
    /// The agent is about to read a file of this session's own folder of reviews, where a review
    /// too long to ride inline is written: the app allows the read, so the CLI asks nothing.
    /// A read of another session's review is not this.
    ReadOwnFile,
    OwnTool,
    Reply(AgentReply),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asked {
    pub session: CliSession,
    pub terminal: Option<String>,
    pub folder: Option<PathBuf>,
    pub asks: Asks,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentReply {
    pub thread: String,
    pub text: String,
    /// Only the reviewer resolves a thread: the reply tool has no way to.
    pub addressed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer<'a> {
    Allow,
    Replied {
        thread: &'a str,
        addressed: bool,
        characters: usize,
    },
    Refused {
        why: &'a str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rendered<'a> {
    pub text: &'a str,
    pub review: &'a str,
    pub round: u32,
    pub threads: usize,
}

pub trait Delivery {
    fn instruction(&self) -> &'static str;

    /// The comments count as sent only once this succeeded.
    fn hand_out(&mut self, rendered: &Rendered<'_>) -> io::Result<Vec<u8>>;
}

/// An agent the app cannot deliver to has no reply tool, so it is asked to answer each thread in
/// its own answer, naming it by id.
pub const COPY_INSTRUCTION: &str = "This is a code review. Address each thread below. \
In your answer, reply to each thread on its own, starting with its id as it is written \
after \"Thread\", and say whether you addressed it.";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CopyAsText;

impl Delivery for CopyAsText {
    fn instruction(&self) -> &'static str {
        COPY_INSTRUCTION
    }

    fn hand_out(&mut self, rendered: &Rendered<'_>) -> io::Result<Vec<u8>> {
        Ok(rendered.text.as_bytes().to_vec())
    }
}

/// The Keeper calls it from one thread at a time.
pub trait Adapter: Send {
    /// The name the Link and the Store know the CLI by: `claude` (not the display title).
    fn name(&self) -> &'static str;

    fn title(&self) -> &'static str;

    /// What a launcher runs in the user's shell, as typed: `claude`.
    fn command(&self) -> &'static str;

    /// Writes the CLI's pieces into `folder` and gives the entries every shell of the app gets
    /// so the CLI loads them; called at every Keeper start. Writes nothing outside `folder`:
    /// the user's CLI settings change only through `answer_consent`, after the user's yes.
    fn install(&self, folder: &Path, companions: &Path) -> io::Result<Vec<ShellEntry>>;

    /// How the user's leave to write into their own settings of the CLI stands: None for an
    /// Adapter that never writes there, and while its CLI is not installed. Reads only.
    fn consent(&self, folder: &Path) -> Option<Consent> {
        let _ = folder;
        None
    }

    fn answer_consent(&self, folder: &Path, answer: ConsentAnswer) -> Option<Consent> {
        let _ = (folder, answer);
        None
    }

    /// Byte runs in a terminal's output the Keeper must hear of, as the end of an interrupted
    /// turn. The Dock watches every terminal for them; `marked` is called with the index seen.
    fn marks(&self) -> Vec<Vec<u8>>;

    fn report(&mut self, report: &Report<'_>) -> Result<Vec<Reported>, ReportError>;

    fn marked(&mut self, terminal: &str, mark: usize) -> Vec<Reported>;

    fn asked(&self, folder: &Path, report: &Report<'_>) -> Result<Option<Asked>, ReportError>;

    fn instruction(&self) -> &'static str;

    fn delivery<'a>(&'a self, folder: &'a Path, asked: &'a Asked) -> Box<dyn Delivery + 'a>;

    fn answer(&self, asked: &Asked, answer: &Answer<'_>) -> Vec<u8>;

    fn resume(&self, session: &CliSession, cli_record: Option<&Path>) -> String;

    /// `written_before` is None for the clean-up the user asks for, else now less the set keep
    /// time: a session's files go only when all are older. Gives the bytes removed, as disk blocks.
    fn remove_files_of_ended(
        &self,
        folder: &Path,
        runs: &dyn Fn(&str) -> bool,
        written_before: Option<std::time::SystemTime>,
    ) -> io::Result<u64> {
        let _ = (folder, runs, written_before);
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_hands_the_rendered_text_back_as_it_is_after_its_own_instruction() {
        let text = format!("{COPY_INSTRUCTION}\n\n## Thread t1\n\nComment:\nwhy `this`?");
        let mut copy = CopyAsText;
        assert_eq!(copy.instruction(), COPY_INSTRUCTION);
        let handed = copy
            .hand_out(&Rendered {
                text: &text,
                review: "r1",
                round: 1,
                threads: 1,
            })
            .expect("a copy cannot fail");
        assert_eq!(handed, text.as_bytes());
    }
}

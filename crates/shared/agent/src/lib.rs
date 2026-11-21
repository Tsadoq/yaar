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
//!   came from, and the CLI's own record of the session; [`CliSession`], the CLI's
//!   own name for a session;
//! - [`Report`], what a piece of the app inside the CLI passed on, as the Keeper hands it
//!   to the Adapter;
//! - [`Adapter`], what one integration module of a CLI supplies: its names, the pieces it
//!   writes into the app's folder and the entries every shell of the app gets for them,
//!   the marks of the terminal output it watches for, the reading of its reports, its
//!   delivery and its command for bringing a session back; [`ShellEntry`] and
//!   [`ReportError`]; and, for an Adapter that writes into the user's own settings of its
//!   CLI only with the user's leave, its [`Consent`] and the [`ConsentAnswer`] it keeps;
//! - what a report waits for the app to answer, [`Asked`] with its [`Asks`] and the
//!   agent's [`AgentReply`] to a thread; and what the app answers it, beyond a review,
//!   [`Answer`], which the Adapter writes in its CLI's own words;
//! - [`Delivery`], one way of delivering a review, handed the review as [`Rendered`]; and
//!   [`CopyAsText`], the way of a review copied by hand, with its [`COPY_INSTRUCTION`].
//!
//! Every event names its session, and a write made with the CLI's edit tool names its
//! path and the lines it changed: a marker per agent in Flow, after the first release,
//! needs all three.
//!
//! A way of delivering a review is one delivery module (the seam of NFR-003): a value
//! that implements [`Delivery`], which says how the agent answers each thread
//! ([`Delivery::instruction`]) and hands the rendered review out in its own way
//! ([`Delivery::hand_out`]). Claude Code's hook is one, which its Adapter gives for each
//! message that carries a review ([`Adapter::delivery`]); pi's extension is another; a
//! copy as text is the third ([`CopyAsText`]). The review itself is rendered by the Review
//! engine, which gives every way the same text after that way's instruction and knows no
//! way by name; nothing here knows the text's form.
//!
//! Nothing here knows the Link, the Store or the Window. The Keeper program lists the
//! Adapters, and no other crate of the app names one.

use std::ffi::OsString;
use std::fmt;
use std::io;
use std::ops::RangeInclusive;
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
        /// The lines of the file as it is now that the write changed, numbered from 1, in
        /// order: for each part the edit tool replaced, the lines that took its place, or
        /// the one line where a part that left nothing in its place was. None for a file
        /// written whole, and where the CLI did not say.
        lines: Option<Vec<RangeInclusive<u32>>>,
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
    /// The folder the CLI works in, as it reported it: for Claude Code the `cwd` of the
    /// hook's input. The Keeper finds the session's workspace from it, and from the
    /// terminal where it names none.
    pub folder: Option<PathBuf>,
    /// The CLI's own record of the session, a file, as its report named it: for pi the
    /// session file, which its command for bringing the session back needs. None where
    /// the report names none, and for a CLI whose resume needs only the session's id, as
    /// Claude Code's. The Keeper keeps the last one a session's reports named, so that
    /// [`Adapter::resume`] is given it after a restart.
    pub cli_record: Option<PathBuf>,
    /// What happened.
    pub event: Event,
}

/// What a piece of the app inside a CLI passed on, as the Keeper hands it to the
/// Adapter of that CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report<'a> {
    /// The terminal of the app the CLI runs in, as the piece's environment named it.
    pub terminal: Option<&'a str>,
    /// The process of the agent CLI, as the piece found it: for Claude Code's hook
    /// program, its parent.
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
    /// A variable set to this value, whatever the user's own environment holds: pi's
    /// `YAAR_PI_REPORT`, which names the program pi's extension reports through.
    Variable {
        /// The variable.
        name: OsString,
        /// Its value.
        value: OsString,
    },
}

/// What the user said when an Adapter asked leave to write into the user's own settings of
/// its CLI, as the Adapter keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentAnswer {
    /// Not asked yet, or asked and not answered.
    NotAsked,
    /// Yes: the Adapter's entry is in the CLI's settings.
    Yes,
    /// No: nothing of the app's is in the CLI's settings.
    No,
}

/// How an Adapter that writes into the user's own settings of its CLI only with the user's
/// leave stands: what it asks, what the user said, and what went wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Consent {
    /// What the Window asks the user, in plain words that name what the app will write and
    /// where, and say that the answer can be changed.
    pub question: String,
    /// What the user said, as the CLI's settings show it now: a yes whose entry is no
    /// longer there is [`ConsentAnswer::NotAsked`] again.
    pub answer: ConsentAnswer,
    /// What went wrong the last time the Adapter read the CLI's settings or changed them,
    /// as a sentence a person reads: a settings file the Adapter cannot read as the CLI
    /// writes it, which it then left as it was. None when nothing went wrong.
    pub problem: Option<String>,
}

/// Why a report could not be read. The Keeper has answered the piece already, and writes
/// this down; the CLI goes on.
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

/// What a report of a CLI's piece waits for the app to answer, beyond its events: the
/// moments at which the piece hands its CLI something of the app's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asks {
    /// The user submitted a message, which the CLI hands the model now, a message the CLI
    /// queued while the agent worked among them: what is attached for the session rides
    /// with it. A message the CLI starts itself, and the answer to a permission prompt or
    /// any other dialog, are never this.
    Message,
    /// The agent is about to read a file of the Adapter's folder of reviews of this
    /// session, where its delivery writes a review too long to ride inline: the app allows
    /// the read, so that the CLI asks the user nothing. Any other read is not this, a read
    /// of another session's review among them.
    ReadOwnFile,
    /// The agent is about to call the app's own reply tool: the app allows the call, so
    /// that the CLI asks the user nothing.
    OwnTool,
    /// The agent replied to a thread through the reply tool.
    Reply(AgentReply),
}

/// What a report waits for, and for which session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asked {
    /// The CLI's own name for the session the report is of.
    pub session: CliSession,
    /// The terminal of the app the CLI runs in, as the piece's environment named it; none
    /// outside the app's terminals.
    pub terminal: Option<String>,
    /// The folder the CLI works in, as the report named it.
    pub folder: Option<PathBuf>,
    /// What it waits for.
    pub asks: Asks,
}

/// A reply of the agent to one thread, through the app's reply tool, as the agent wrote
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentReply {
    /// The thread, by the id the review the agent received names it with.
    pub thread: String,
    /// The reply, byte for byte as the agent wrote it.
    pub text: String,
    /// Whether the agent says it addressed the thread. Only the reviewer resolves a
    /// thread: the tool has no way to.
    pub addressed: bool,
}

/// What the app answers a report that waits for something other than a review, for the
/// Adapter to hand its piece in the CLI's own words. A review rides through the CLI's
/// [`Delivery`] instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer<'a> {
    /// The read, or the call of the app's own tool, is allowed.
    Allow,
    /// The reply was kept in the thread.
    Replied {
        /// The thread.
        thread: &'a str,
        /// Whether it was marked addressed.
        addressed: bool,
        /// How long the reply is, in characters.
        characters: usize,
    },
    /// The reply was refused, for this reason, as a sentence the agent reads: a thread
    /// the session's review does not hold, or the app not running.
    Refused {
        /// Why.
        why: &'a str,
    },
}

/// A review as the Review engine rendered it for one send, handed to a [`Delivery`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rendered<'a> {
    /// Everything the agent is to receive: the delivery's own instruction first, as
    /// [`Delivery::instruction`] gave it, then every unsent comment the send carries.
    pub text: &'a str,
    /// The review, by its id.
    pub review: &'a str,
    /// The round the send starts.
    pub round: u32,
    /// How many threads the text holds.
    pub threads: usize,
}

/// One way of delivering a review: a delivery module. It says how the agent is to answer
/// each thread, and hands the rendered review to wherever it goes. Adding a way is adding
/// one of these and changing nothing that renders, sends or keeps a review.
pub trait Delivery {
    /// What tells the agent how to answer each thread of the review: it goes first in the
    /// text, whatever the user's message says, and asks for nothing that resolves a
    /// thread.
    fn instruction(&self) -> &'static str;

    /// Hands `rendered` out whole, whatever its length, and gives the bytes that go back
    /// to whoever asked: for Claude Code the output of the hook that carries it, for a
    /// copy the text itself. The comments count as sent only once this succeeded; a
    /// failure sends nothing.
    fn hand_out(&mut self, rendered: &Rendered<'_>) -> io::Result<Vec<u8>>;
}

/// The instruction that goes first in a review copied as text. An agent the app cannot
/// deliver to has no reply tool, so it is asked to answer each thread in its own answer,
/// naming the thread by its id.
pub const COPY_INSTRUCTION: &str = "This is a code review. Address each thread below. \
In your answer, reply to each thread on its own, starting with its id as it is written \
after \"Thread\", and say whether you addressed it.";

/// Copying a review as text, for the user to paste to an agent the app cannot deliver
/// to: the text, after [`COPY_INSTRUCTION`], is handed back as it is.
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
    /// was written before is written again, at the same paths. Nothing is written anywhere
    /// else: not into a worktree, and not into the user's settings of the CLI, which only
    /// [`Adapter::answer_consent`] writes, after the user's yes.
    fn install(&self, folder: &Path, companions: &Path) -> io::Result<Vec<ShellEntry>>;

    /// Whether this Adapter writes into the user's own settings of its CLI, which it does
    /// only with the user's leave, and how that stands now: none for an Adapter that never
    /// does, as Claude Code's, and none while its CLI is not installed, when there is
    /// nothing to ask. `folder` is the Adapter's own folder of the app, where it keeps the
    /// user's answer. It reads, and writes nothing.
    fn consent(&self, folder: &Path) -> Option<Consent> {
        let _ = folder;
        None
    }

    /// Takes the user's answer and keeps it in `folder`. A yes writes the Adapter's one
    /// entry into the CLI's settings and changes nothing else there; a no, or
    /// [`ConsentAnswer::NotAsked`] (ask again), takes out the entry if it is there and
    /// writes nothing else. A settings file the Adapter cannot read as the CLI writes it
    /// is left as it was, and the [`Consent`] given says so in `problem`. Gives how it
    /// stands then, as [`Adapter::consent`] would; none for an Adapter that needs no leave
    /// or whose CLI is not installed, which keeps nothing.
    fn answer_consent(&self, folder: &Path, answer: ConsentAnswer) -> Option<Consent> {
        let _ = (folder, answer);
        None
    }

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

    /// What the same report [`Adapter::report`] was just given waits for the app to
    /// answer; none for a report that waits for nothing, which is answered with nothing
    /// for the CLI. A report from the app's reply tool gives no event, only this. `folder`
    /// is the Adapter's own folder of the app, where [`Adapter::install`] wrote its pieces
    /// and its delivery writes long reviews. It changes nothing the Adapter remembers.
    fn asked(&self, folder: &Path, report: &Report<'_>) -> Result<Option<Asked>, ReportError>;

    /// What tells the agent how to answer each thread of a review it receives: with this
    /// CLI's reply tool, by its name as the CLI lists it to the agent, one call for each
    /// thread, with the thread's id, the reply and whether the change addressed it. The
    /// [`Delivery::instruction`] of every delivery [`Adapter::delivery`] gives, and so the
    /// first thing of every review delivered to the CLI, whatever the user's message says;
    /// it asks for nothing that resolves a thread.
    fn instruction(&self) -> &'static str;

    /// The delivery of a review that rides with the message `asked` stands for
    /// ([`Asks::Message`]): what the piece hands its CLI, as bytes. A review too long to
    /// ride inline is written under `folder`, the Adapter's own folder of the app,
    /// outside every repository, in a folder of that session's own, whose files the
    /// Adapter then allows the session to read ([`Asks::ReadOwnFile`]).
    fn delivery<'a>(&'a self, folder: &'a Path, asked: &'a Asked) -> Box<dyn Delivery + 'a>;

    /// What the piece of `asked` hands its CLI for `answer`, as bytes: for Claude Code the
    /// output of a hook that allows, or the result of a call of the reply tool.
    fn answer(&self, asked: &Asked, answer: &Answer<'_>) -> Vec<u8>;

    /// What a terminal runs in the user's shell to bring back `session`, as the user would
    /// type it: the CLI's own command for that, with the session's id quoted for the
    /// shell, as `claude --resume '<id>'`. `cli_record` is the CLI's own record of the
    /// session, as the session's reports last named it ([`Reported::cli_record`]); none
    /// where they named none. A CLI whose command needs only the id leaves it unread.
    fn resume(&self, session: &CliSession, cli_record: Option<&Path>) -> String;

    /// Removes the files this Adapter wrote under its own `folder` for agent sessions that
    /// no longer run, as the files of reviews too long to ride inline with a message: the
    /// clean-up the user asks for in the settings, and the one the set time makes. `runs`
    /// tells, by the CLI's own id of a session ([`CliSession`]'s text), whether the session
    /// still runs: nothing of a session that runs is removed, since its agent may read the
    /// file still. Removes nothing outside `folder`, and follows no link out of it. Gives
    /// the bytes it removed, as the blocks they used on disk.
    ///
    /// An Adapter that writes no such files removes nothing: the default.
    fn remove_files_of_ended(&self, folder: &Path, runs: &dyn Fn(&str) -> bool) -> io::Result<u64> {
        let _ = (folder, runs);
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

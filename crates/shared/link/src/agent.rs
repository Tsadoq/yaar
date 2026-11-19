//! What the two programs, and the app's pieces inside an agent CLI, say about agents: what
//! a piece reports, the sessions with their states, the turns with their steps, and the
//! history of a worktree with the changes no agent made.
//!
//! A piece of the app inside an agent CLI, as the hook program of Claude Code, speaks the
//! Link too: it says hello, sends one [`crate::Request::Report`] with what the CLI handed
//! it, waits for the answer and leaves. The Keeper hands the report to the Adapter of that
//! CLI, which turns it into the app's own events; a Window hears only what follows from
//! them, as facts.

use std::path::PathBuf;

use crate::LinkError;
use crate::review::{ReviewId, SessionId, TurnId};
use crate::terminal::{Program, TerminalId, read_program, write_program};
use crate::wire::{In, Out};
use crate::workspace::{
    ChangedFile, Commit, WorkspaceId, read_changed_file, read_commit, write_changed_file,
    write_commit,
};

/// What a piece of the app inside an agent CLI reports, as the CLI handed it to the piece.
/// The Keeper reads it through the Adapter named by `agent`; the Link carries it as it
/// came, so that a new CLI needs no new message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentReport {
    /// The Adapter the report is for, by its name: `claude` for Claude Code.
    pub agent: String,
    /// The terminal of the app the CLI runs in, as its environment names it; none for a
    /// CLI that runs outside the app's terminals.
    pub terminal: Option<TerminalId>,
    /// The process of the agent CLI that started the piece: the piece's parent.
    pub pid: u32,
    /// What the CLI handed its piece, byte for byte: for Claude Code the input of the hook.
    pub payload: Vec<u8>,
}

/// What an agent session is doing, as the sidebar and the agent's terminal show it.
///
/// The order of the variants is the order in which a workspace with several sessions
/// shows them: the first that holds for any session is the workspace's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SessionState {
    /// The agent waits at a permission prompt for the user's answer.
    WaitsForPermission,
    /// The agent has finished a turn, and the reviewer has not marked the change reviewed
    /// since.
    Unreviewed,
    /// The agent works on a turn.
    Works,
    /// The agent runs and waits for the user's next message, with nothing unreviewed.
    Idle,
    /// The session has ended, or its process is gone; its review stays.
    NotRunning,
}

/// One session of an agent CLI, as the Keeper knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// Its id.
    pub id: SessionId,
    /// The Adapter of its CLI, by its name: `claude`.
    pub agent: String,
    /// The name the Window shows for it: the CLI's name and the number of the session
    /// among that CLI's sessions in the workspace, `Claude Code 2`.
    pub title: String,
    /// The workspace it works in.
    pub workspace: WorkspaceId,
    /// The terminal it runs in; none once it runs no more, or when it reported from
    /// outside the app's terminals.
    pub terminal: Option<TerminalId>,
    /// What it is doing.
    pub state: SessionState,
    /// The review that belongs to it; none once the next session of its CLI took that
    /// review over.
    pub review: Option<ReviewId>,
}

/// The agent sessions of one workspace, and the state the workspace shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceAgents {
    /// The workspace.
    pub workspace: WorkspaceId,
    /// The state the sidebar shows for it: the first of [`SessionState`]'s order that
    /// holds for any of its sessions; none when no session of a supported agent CLI ever
    /// reported in it.
    pub shown: Option<SessionState>,
    /// Its sessions: those that run, and those that ended and still hold a review, in the
    /// order they started.
    pub sessions: Vec<Session>,
}

/// One step of a turn: what the agent read, changed or ran, in the order it happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// The agent read a file with its own tool.
    Read {
        /// The path, relative to the folder of the worktree when it lies inside it,
        /// absolute otherwise.
        path: PathBuf,
    },
    /// The agent changed a file with its edit tool.
    Edit {
        /// The path, as for [`Step::Read`].
        path: PathBuf,
    },
    /// The agent wrote a file whole with its write tool, a new file among them.
    Write {
        /// The path, as for [`Step::Read`].
        path: PathBuf,
    },
    /// The agent ran a shell command. The files it wrote are not named here: the watcher
    /// of the worktree sees them.
    Command {
        /// The command, as the agent wrote it.
        command: String,
    },
}

/// How a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnEnd {
    /// The agent finished its answer.
    Finished,
    /// The user interrupted the turn, or denied a permission the turn waited for.
    Interrupted,
}

/// One turn of a session, with its steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    /// Its id.
    pub id: TurnId,
    /// The session it belongs to.
    pub session: SessionId,
    /// Its number among the turns of the session, from 1.
    pub number: u32,
    /// How it ended; none while it runs.
    pub ended: Option<TurnEnd>,
    /// The state of the worktree recorded when it ended, by its full id; none while it
    /// runs, and when the recording failed.
    pub state: Option<String>,
    /// Its steps, in the order they happened.
    pub steps: Vec<Step>,
}

/// One entry of the history of a worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryEntry {
    /// A commit of the branch.
    Commit(Commit),
    /// A turn of an agent that ended, with the state recorded at its end.
    Turn {
        /// The turn.
        turn: TurnId,
        /// Its session.
        session: SessionId,
        /// The state recorded when it ended, by its full id.
        state: String,
    },
}

/// A changed file that is not committed, and who made the change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditedFile {
    /// The file, as the changes list it.
    pub file: ChangedFile,
    /// The sessions it is credited to; empty when no agent made the change, as for an
    /// edit made outside the app or by an agent CLI the app does not support, and for a
    /// file two sessions wrote through shell commands at once.
    pub by: Vec<SessionId>,
}

/// The history of a worktree: what Changes shows as its tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    /// The workspace.
    pub workspace: WorkspaceId,
    /// The version the first entry is counted from, by its full id: the commit the changes
    /// of the workspace are counted from. None where nothing is committed yet.
    pub start: Option<String>,
    /// The commits of the branch since `start` and the turns that ended on them, oldest
    /// first, in the order they happened.
    pub entries: Vec<HistoryEntry>,
    /// The changes that are not committed, from the commit checked out to the files on
    /// disk, ordered by path, each with who made it.
    pub uncommitted: Vec<CreditedFile>,
}

/// A terminal the Keeper holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalInfo {
    /// Its id.
    pub id: TerminalId,
    /// The workspace it was opened for.
    pub workspace: WorkspaceId,
    /// What it was opened to run.
    pub program: Program,
}

pub(crate) fn write_report(out: &mut Out, report: &AgentReport) {
    out.text(&report.agent);
    out.optional(report.terminal.as_ref(), |out, terminal| {
        out.text(terminal.as_str())
    });
    out.u32(report.pid);
    out.bytes(&report.payload);
}

pub(crate) fn read_report(input: &mut In<'_>) -> Result<AgentReport, LinkError> {
    Ok(AgentReport {
        agent: input.text()?,
        terminal: input.optional(|input| Ok(TerminalId::new(input.text()?)))?,
        pid: input.u32()?,
        payload: input.bytes()?,
    })
}

fn write_state(out: &mut Out, state: SessionState) {
    out.u8(match state {
        SessionState::WaitsForPermission => 0,
        SessionState::Unreviewed => 1,
        SessionState::Works => 2,
        SessionState::Idle => 3,
        SessionState::NotRunning => 4,
    });
}

fn read_state(input: &mut In<'_>) -> Result<SessionState, LinkError> {
    Ok(match input.u8()? {
        0 => SessionState::WaitsForPermission,
        1 => SessionState::Unreviewed,
        2 => SessionState::Works,
        3 => SessionState::Idle,
        4 => SessionState::NotRunning,
        _ => return Err(LinkError::Malformed("an unknown state of a session")),
    })
}

fn write_session(out: &mut Out, session: &Session) {
    out.text(session.id.as_str());
    out.text(&session.agent);
    out.text(&session.title);
    out.text(session.workspace.as_str());
    out.optional(session.terminal.as_ref(), |out, terminal| {
        out.text(terminal.as_str())
    });
    write_state(out, session.state);
    out.optional(session.review.as_ref(), |out, review| {
        out.text(review.as_str())
    });
}

fn read_session(input: &mut In<'_>) -> Result<Session, LinkError> {
    Ok(Session {
        id: SessionId::new(input.text()?),
        agent: input.text()?,
        title: input.text()?,
        workspace: WorkspaceId::new(input.text()?),
        terminal: input.optional(|input| Ok(TerminalId::new(input.text()?)))?,
        state: read_state(input)?,
        review: input.optional(|input| Ok(ReviewId::new(input.text()?)))?,
    })
}

pub(crate) fn write_workspace_agents(out: &mut Out, agents: &WorkspaceAgents) {
    out.text(agents.workspace.as_str());
    out.optional(agents.shown.as_ref(), |out, state| write_state(out, *state));
    out.list(&agents.sessions, write_session);
}

pub(crate) fn read_workspace_agents(input: &mut In<'_>) -> Result<WorkspaceAgents, LinkError> {
    Ok(WorkspaceAgents {
        workspace: WorkspaceId::new(input.text()?),
        shown: input.optional(read_state)?,
        sessions: input.list(read_session)?,
    })
}

pub(crate) fn write_step(out: &mut Out, step: &Step) {
    match step {
        Step::Read { path } => {
            out.u8(0);
            out.path(path);
        }
        Step::Edit { path } => {
            out.u8(1);
            out.path(path);
        }
        Step::Write { path } => {
            out.u8(2);
            out.path(path);
        }
        Step::Command { command } => {
            out.u8(3);
            out.text(command);
        }
    }
}

pub(crate) fn read_step(input: &mut In<'_>) -> Result<Step, LinkError> {
    Ok(match input.u8()? {
        0 => Step::Read {
            path: input.path()?,
        },
        1 => Step::Edit {
            path: input.path()?,
        },
        2 => Step::Write {
            path: input.path()?,
        },
        3 => Step::Command {
            command: input.text()?,
        },
        _ => return Err(LinkError::Malformed("an unknown kind of step")),
    })
}

fn write_turn_end(out: &mut Out, end: TurnEnd) {
    out.u8(match end {
        TurnEnd::Finished => 0,
        TurnEnd::Interrupted => 1,
    });
}

fn read_turn_end(input: &mut In<'_>) -> Result<TurnEnd, LinkError> {
    Ok(match input.u8()? {
        0 => TurnEnd::Finished,
        1 => TurnEnd::Interrupted,
        _ => return Err(LinkError::Malformed("a turn ended in an unknown way")),
    })
}

pub(crate) fn write_turn(out: &mut Out, turn: &Turn) {
    out.text(turn.id.as_str());
    out.text(turn.session.as_str());
    out.u32(turn.number);
    out.optional(turn.ended.as_ref(), |out, end| write_turn_end(out, *end));
    out.optional(turn.state.as_deref(), Out::text);
    out.list(&turn.steps, write_step);
}

pub(crate) fn read_turn(input: &mut In<'_>) -> Result<Turn, LinkError> {
    Ok(Turn {
        id: TurnId::new(input.text()?),
        session: SessionId::new(input.text()?),
        number: input.u32()?,
        ended: input.optional(read_turn_end)?,
        state: input.optional(In::text)?,
        steps: input.list(read_step)?,
    })
}

fn write_entry(out: &mut Out, entry: &HistoryEntry) {
    match entry {
        HistoryEntry::Commit(commit) => {
            out.u8(0);
            write_commit(out, commit);
        }
        HistoryEntry::Turn {
            turn,
            session,
            state,
        } => {
            out.u8(1);
            out.text(turn.as_str());
            out.text(session.as_str());
            out.text(state);
        }
    }
}

fn read_entry(input: &mut In<'_>) -> Result<HistoryEntry, LinkError> {
    Ok(match input.u8()? {
        0 => HistoryEntry::Commit(read_commit(input)?),
        1 => HistoryEntry::Turn {
            turn: TurnId::new(input.text()?),
            session: SessionId::new(input.text()?),
            state: input.text()?,
        },
        _ => return Err(LinkError::Malformed("an unknown kind of history entry")),
    })
}

fn write_credited(out: &mut Out, credited: &CreditedFile) {
    write_changed_file(out, &credited.file);
    out.list(&credited.by, |out, session| out.text(session.as_str()));
}

fn read_credited(input: &mut In<'_>) -> Result<CreditedFile, LinkError> {
    Ok(CreditedFile {
        file: read_changed_file(input)?,
        by: input.list(|input| Ok(SessionId::new(input.text()?)))?,
    })
}

pub(crate) fn write_history(out: &mut Out, history: &History) {
    out.text(history.workspace.as_str());
    out.optional(history.start.as_deref(), Out::text);
    out.list(&history.entries, write_entry);
    out.list(&history.uncommitted, write_credited);
}

pub(crate) fn read_history(input: &mut In<'_>) -> Result<History, LinkError> {
    Ok(History {
        workspace: WorkspaceId::new(input.text()?),
        start: input.optional(In::text)?,
        entries: input.list(read_entry)?,
        uncommitted: input.list(read_credited)?,
    })
}

pub(crate) fn write_terminal_info(out: &mut Out, terminal: &TerminalInfo) {
    out.text(terminal.id.as_str());
    out.text(terminal.workspace.as_str());
    write_program(out, &terminal.program);
}

pub(crate) fn read_terminal_info(input: &mut In<'_>) -> Result<TerminalInfo, LinkError> {
    Ok(TerminalInfo {
        id: TerminalId::new(input.text()?),
        workspace: WorkspaceId::new(input.text()?),
        program: read_program(input)?,
    })
}

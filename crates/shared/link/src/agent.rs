//! What the two programs, and the app's pieces inside an agent CLI, say about agents: what
//! a piece reports, the sessions with their states, the turns with their steps, and the
//! history of a worktree with the changes no agent made.
//!
//! A piece of the app inside an agent CLI, as the hook program of Claude Code, speaks the
//! Link too: it says hello, sends one [`crate::Request::Report`] with what the CLI handed
//! it, waits for the answer and leaves. The Keeper hands the report to the Adapter of that
//! CLI, which turns it into the app's own events; a Window hears only what follows from
//! them, as facts.

use std::ops::RangeInclusive;
use std::path::PathBuf;

use crate::LinkError;
use crate::review::{Attachment, ReviewId, SessionId, TurnId, read_attachment, write_attachment};
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
    /// The process of the agent CLI, as the piece found it: for the hook program of
    /// Claude Code, its parent.
    pub pid: u32,
    /// What the CLI handed its piece, byte for byte: for Claude Code the input of the hook.
    pub payload: Vec<u8>,
}

/// What an agent session is doing, as the sidebar and the agent's terminal show it.
///
/// The order of the variants is the order in which a workspace with several sessions
/// shows them: the first that holds for any session is the workspace's. Idle comes last:
/// a workspace shows it only when every session is idle, so that the four states the
/// sidebar marks keep their own order whatever else runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SessionState {
    /// The agent waits at a permission prompt for the user's answer.
    WaitsForPermission,
    /// The agent has finished a turn, and the reviewer has not marked the change reviewed
    /// since.
    Unreviewed,
    /// The agent works on a turn.
    Works,
    /// The session has ended, or its process is gone; its review stays.
    NotRunning,
    /// The agent runs and waits for the user's next message, with nothing unreviewed.
    Idle,
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
    /// Its turn that runs, from the turn's start to its end; none between turns. A
    /// Window that follows the agent learns the turn's id here as it starts.
    pub turn: Option<TurnId>,
    /// The review that belongs to it; none once the next session of its CLI took that
    /// review over.
    pub review: Option<ReviewId>,
    /// What of that review goes with the user's next message to the session: the review,
    /// or one thread of it alone; none while nothing is attached. The agent's terminal
    /// shows it, with the controls that view and detach it.
    pub attached: Option<Attachment>,
    /// Whether the turn that runs began with a message of the user: false for a turn the
    /// CLI started by itself, as Claude Code does when a background task ends, and while
    /// no turn runs. Only a message of the user switches the review pane to Flow.
    pub turn_from_user: bool,
    /// Whether the session ran when its Keeper stopped, as at a restart of the machine,
    /// and has not run since: the Window offers to resume it, with its CLI's own command
    /// for bringing a session back.
    pub resumable: bool,
}

/// An agent CLI the app supports, as the Window offers to start it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCli {
    /// Its Adapter, by its name: `claude`. A launcher of it is
    /// [`crate::Program::Agent`] or [`crate::Program::AgentFor`] with this name.
    pub agent: String,
    /// The name a person reads: `Claude Code`.
    pub title: String,
}

/// What the user said when an Adapter asked leave to write into the user's own settings
/// of its CLI, as the Keeper keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentAnswer {
    /// Not asked yet, or asked and not answered: the Window shows the question.
    NotAsked,
    /// Yes: the Adapter's entry is in the CLI's settings.
    Yes,
    /// No: the app writes nothing into the CLI's settings, and takes out what it wrote.
    No,
}

/// One Adapter that writes into the user's own settings of its CLI only with the user's
/// leave, and how that stands. Only an Adapter that needs leave, and whose CLI is
/// installed, is listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Consent {
    /// The Adapter, by its name: `pi`.
    pub agent: String,
    /// What the Window asks the user, in the Adapter's words: what the app will write and
    /// where, and that the answer can be changed. The Window shows it as it is.
    pub question: String,
    /// What the user said.
    pub answer: ConsentAnswer,
    /// What went wrong the last time the app looked at the CLI's settings or changed them,
    /// in a sentence the Window shows as it is: a settings file the app cannot read as the
    /// CLI writes it, which the app then left as it was. None when nothing went wrong.
    pub problem: Option<String>,
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
        /// The lines of the file after the edit that it changed, numbered from 1, in
        /// order; none where the CLI did not say.
        lines: Option<Vec<RangeInclusive<u32>>>,
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
    /// The state the turn's change counts from, by its full id: the newest state of the
    /// worktree recorded before the turn started, which the live diff of Flow compares
    /// with the files on disk while the turn runs, and with [`Turn::state`] once it ended.
    /// A turn that started while the state of the turn before was still being recorded
    /// counts from that state: none until it is kept, which is before the
    /// [`crate::Fact::TurnEnded`] of the turn before is told, so a Window that hears that
    /// fact and asks again reads it. None where the worktree had no recorded state then:
    /// the change then counts from the commit checked out.
    pub start: Option<String>,
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
        /// The state recorded when it ended, by its full id; none when the recording
        /// failed or the state is no longer kept. A Window shows such a turn and does not
        /// let it be selected.
        state: Option<String>,
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

/// The history of a worktree: what Changes shows as its tree, but for its last entry,
/// the uncommitted changes, which [`crate::Request::GetUncommitted`] reads. It is read
/// from what the Keeper keeps and from the commits, without asking git what changed on
/// disk, so that a window that opens draws it at once.
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
        SessionState::NotRunning => 3,
        SessionState::Idle => 4,
    });
}

fn read_state(input: &mut In<'_>) -> Result<SessionState, LinkError> {
    Ok(match input.u8()? {
        0 => SessionState::WaitsForPermission,
        1 => SessionState::Unreviewed,
        2 => SessionState::Works,
        3 => SessionState::NotRunning,
        4 => SessionState::Idle,
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
    out.optional(session.turn.as_ref(), |out, turn| out.text(turn.as_str()));
    out.optional(session.review.as_ref(), |out, review| {
        out.text(review.as_str())
    });
    out.optional(session.attached.as_ref(), write_attachment);
    write_flag(out, session.turn_from_user);
    write_flag(out, session.resumable);
}

fn read_session(input: &mut In<'_>) -> Result<Session, LinkError> {
    Ok(Session {
        id: SessionId::new(input.text()?),
        agent: input.text()?,
        title: input.text()?,
        workspace: WorkspaceId::new(input.text()?),
        terminal: input.optional(|input| Ok(TerminalId::new(input.text()?)))?,
        state: read_state(input)?,
        turn: input.optional(|input| Ok(TurnId::new(input.text()?)))?,
        review: input.optional(|input| Ok(ReviewId::new(input.text()?)))?,
        attached: input.optional(read_attachment)?,
        turn_from_user: read_flag(input)?,
        resumable: read_flag(input)?,
    })
}

fn write_flag(out: &mut Out, flag: bool) {
    out.u8(u8::from(flag));
}

fn read_flag(input: &mut In<'_>) -> Result<bool, LinkError> {
    match input.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(LinkError::Malformed("a flag is neither 0 nor 1")),
    }
}

pub(crate) fn write_agent_cli(out: &mut Out, cli: &AgentCli) {
    out.text(&cli.agent);
    out.text(&cli.title);
}

pub(crate) fn read_agent_cli(input: &mut In<'_>) -> Result<AgentCli, LinkError> {
    Ok(AgentCli {
        agent: input.text()?,
        title: input.text()?,
    })
}

pub(crate) fn write_consent(out: &mut Out, consent: &Consent) {
    out.text(&consent.agent);
    out.text(&consent.question);
    write_consent_answer(out, consent.answer);
    out.optional(consent.problem.as_deref(), Out::text);
}

pub(crate) fn read_consent(input: &mut In<'_>) -> Result<Consent, LinkError> {
    Ok(Consent {
        agent: input.text()?,
        question: input.text()?,
        answer: read_consent_answer(input)?,
        problem: input.optional(In::text)?,
    })
}

pub(crate) fn write_consent_answer(out: &mut Out, answer: ConsentAnswer) {
    out.u8(match answer {
        ConsentAnswer::NotAsked => 0,
        ConsentAnswer::Yes => 1,
        ConsentAnswer::No => 2,
    });
}

pub(crate) fn read_consent_answer(input: &mut In<'_>) -> Result<ConsentAnswer, LinkError> {
    match input.u8()? {
        0 => Ok(ConsentAnswer::NotAsked),
        1 => Ok(ConsentAnswer::Yes),
        2 => Ok(ConsentAnswer::No),
        _ => Err(LinkError::Malformed(
            "unknown answer to a question for leave",
        )),
    }
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
        Step::Edit { path, lines } => {
            out.u8(1);
            out.path(path);
            out.optional(lines.as_ref(), |out, lines| {
                out.list(lines, |out, range| {
                    out.u32(*range.start());
                    out.u32(*range.end());
                });
            });
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
            lines: input.optional(|input| {
                input.list(|input| {
                    let start = input.u32()?;
                    let end = input.u32()?;
                    if end < start {
                        return Err(LinkError::Malformed(
                            "a range of lines ends before it starts",
                        ));
                    }
                    Ok(start..=end)
                })
            })?,
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
    out.optional(turn.start.as_deref(), Out::text);
    out.optional(turn.state.as_deref(), Out::text);
    out.list(&turn.steps, write_step);
}

pub(crate) fn read_turn(input: &mut In<'_>) -> Result<Turn, LinkError> {
    Ok(Turn {
        id: TurnId::new(input.text()?),
        session: SessionId::new(input.text()?),
        number: input.u32()?,
        ended: input.optional(read_turn_end)?,
        start: input.optional(In::text)?,
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
            out.optional(state.as_deref(), Out::text);
        }
    }
}

fn read_entry(input: &mut In<'_>) -> Result<HistoryEntry, LinkError> {
    Ok(match input.u8()? {
        0 => HistoryEntry::Commit(read_commit(input)?),
        1 => HistoryEntry::Turn {
            turn: TurnId::new(input.text()?),
            session: SessionId::new(input.text()?),
            state: input.optional(In::text)?,
        },
        _ => return Err(LinkError::Malformed("an unknown kind of history entry")),
    })
}

pub(crate) fn write_credited(out: &mut Out, credited: &CreditedFile) {
    write_changed_file(out, &credited.file);
    out.list(&credited.by, |out, session| out.text(session.as_str()));
}

pub(crate) fn read_credited(input: &mut In<'_>) -> Result<CreditedFile, LinkError> {
    Ok(CreditedFile {
        file: read_changed_file(input)?,
        by: input.list(|input| Ok(SessionId::new(input.text()?)))?,
    })
}

pub(crate) fn write_history(out: &mut Out, history: &History) {
    out.text(history.workspace.as_str());
    out.optional(history.start.as_deref(), Out::text);
    out.list(&history.entries, write_entry);
}

pub(crate) fn read_history(input: &mut In<'_>) -> Result<History, LinkError> {
    Ok(History {
        workspace: WorkspaceId::new(input.text()?),
        start: input.optional(In::text)?,
        entries: input.list(read_entry)?,
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

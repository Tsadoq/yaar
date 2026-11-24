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

/// What a piece of the app inside an agent CLI reports, as the CLI handed it. The Keeper reads
/// it through the Adapter named by `agent`, so a new CLI needs no new message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentReport {
    pub agent: String,
    /// None for a CLI that runs outside the app's terminals.
    pub terminal: Option<TerminalId>,
    /// The agent CLI's process: for Claude Code's hook program, its parent.
    pub pid: u32,
    pub payload: Vec<u8>,
}

/// Variant order is the display priority: a workspace shows the first state any session holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SessionState {
    WaitsForPermission,
    Unreviewed,
    Works,
    NotRunning,
    Idle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: SessionId,
    pub agent: String,
    pub title: String,
    pub workspace: WorkspaceId,
    /// None once it runs no more, or when it reported from outside the app's terminals.
    pub terminal: Option<TerminalId>,
    pub state: SessionState,
    pub turn: Option<TurnId>,
    /// None once the next session of its CLI took the review over.
    pub review: Option<ReviewId>,
    pub attached: Option<Attachment>,
    /// False for a turn the CLI started itself (a background task ended) and while no turn runs.
    /// Only a user message switches the review pane to Flow.
    pub turn_from_user: bool,
    /// True when it ran as its Keeper stopped and has not run since: the Window offers to resume
    /// it.
    pub resumable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCli {
    pub agent: String,
    pub title: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentAnswer {
    NotAsked,
    Yes,
    /// The app writes nothing into the CLI's settings and removes what it wrote.
    No,
}

/// An Adapter that writes into its CLI's settings only with the user's leave, and how that
/// stands. Only one that needs leave and whose CLI is installed is listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Consent {
    pub agent: String,
    pub question: String,
    pub answer: ConsentAnswer,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceAgents {
    pub workspace: WorkspaceId,
    pub shown: Option<SessionState>,
    /// Running sessions and ended ones that still hold a review, in start order.
    pub sessions: Vec<Session>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Read {
        /// Relative to the worktree folder when inside it, absolute otherwise.
        path: PathBuf,
    },
    Edit {
        path: PathBuf,
        /// The lines it changed, in the file after the edit, from 1, in order; None where the CLI
        /// did not say.
        lines: Option<Vec<RangeInclusive<u32>>>,
    },
    Write {
        path: PathBuf,
    },
    /// The files it wrote are not named: the worktree watcher sees them.
    Command {
        command: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnEnd {
    Finished,
    /// The user interrupted the turn, or denied a permission it waited for.
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub id: TurnId,
    pub session: SessionId,
    /// Its number among the session's turns, from 1.
    pub number: u32,
    pub ended: Option<TurnEnd>,
    /// Full id of the newest state recorded before the turn started, which its change counts from.
    /// None while the previous turn's state is still being recorded (ask again after that turn's
    /// `Fact::TurnEnded`), or when nothing is recorded: then it counts from the checked-out commit.
    pub start: Option<String>,
    /// Full id of the state recorded at its end; None while it runs or when recording failed.
    pub state: Option<String>,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryEntry {
    Commit(Commit),
    Turn {
        turn: TurnId,
        session: SessionId,
        /// None when recording failed. Shown but not selectable.
        state: Option<String>,
    },
    /// Its state was deleted by the clean-up; stays in the history, not selectable.
    ExpiredTurn {
        turn: TurnId,
        session: SessionId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditedFile {
    pub file: ChangedFile,
    /// Empty when no agent made the change: an outside edit, an unsupported CLI, or a file two
    /// sessions wrote through shell commands at once.
    pub by: Vec<SessionId>,
}

/// The worktree's history as Changes shows it, without the uncommitted changes
/// (`Request::GetUncommitted`). Read from what the Keeper keeps and from the commits, without
/// asking git what changed on disk, so a window that opens draws it at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    pub workspace: WorkspaceId,
    pub start: Option<String>,
    /// Commits since `start` and the turns that ended on them, oldest first.
    pub entries: Vec<HistoryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalInfo {
    pub id: TerminalId,
    pub workspace: WorkspaceId,
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
        HistoryEntry::ExpiredTurn { turn, session } => {
            out.u8(2);
            out.text(turn.as_str());
            out.text(session.as_str());
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
        2 => HistoryEntry::ExpiredTurn {
            turn: TurnId::new(input.text()?),
            session: SessionId::new(input.text()?),
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

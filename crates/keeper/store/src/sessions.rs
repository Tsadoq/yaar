//! What the Store keeps of agents: their sessions, the turns of each with the state
//! recorded at its end, and the steps of each turn, so that a Window that attaches later,
//! and a Keeper started again, find them.
//!
//! Every call that changes something is one transaction that waits for the drive, as the
//! calls of slice 1 do: a step is a few bytes, and an agent takes a few steps a second.

use std::ops::RangeInclusive;
use std::path::PathBuf;

use rusqlite::{OptionalExtension, Row, params};

use crate::reviews::{exists, refusal, refused};
use crate::{ReviewId, Store, StoreError, WorkspaceId, bytes, path};

/// Names one agent session: 32 lowercase hexadecimal digits the Keeper drew.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(pub(crate) String);

impl SessionId {
    /// The id with this text.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The text of the id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Names one turn: 32 lowercase hexadecimal digits the Keeper drew.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TurnId(pub(crate) String);

impl TurnId {
    /// The id with this text.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The text of the id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TurnId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One agent session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    /// Its id.
    pub id: SessionId,
    /// The workspace it works in.
    pub workspace: WorkspaceId,
    /// The name of the Adapter of its CLI: `claude`.
    pub agent: String,
    /// The CLI's own name for the session.
    pub cli_session: String,
    /// The terminal it runs or last ran in, by the id the app gave it.
    pub terminal: Option<String>,
    /// Its number among the sessions of its CLI in the workspace, from 1.
    pub number: u32,
    /// Whether it runs.
    pub running: bool,
    /// When it last stopped running, as the order of ends: a session that ended later has
    /// a larger number. None while it runs, and for one that never ended.
    pub ended: Option<u64>,
    /// How it last stopped running; none while it runs, and for one that never ended.
    pub ended_by: Option<EndedBy>,
}

/// How a session stopped running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndedBy {
    /// Its CLI reported its end: the user quit it, or cleared it for a new session.
    Cli,
    /// Its process went with no end reported, or its terminal ended.
    ProcessGone,
    /// The Keeper stopped while it ran, as when the machine restarted: the session was cut
    /// off, not ended, and is one the app may offer to resume.
    KeeperStopped,
}

/// How a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnEndRecord {
    /// The agent finished its answer.
    Finished,
    /// The user interrupted it.
    Interrupted,
}

/// One turn of a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRecord {
    /// Its id.
    pub id: TurnId,
    /// Its session.
    pub session: SessionId,
    /// Its number in the session, from 1.
    pub number: u32,
    /// How it ended; none while it runs.
    pub ended: Option<TurnEndRecord>,
    /// The full id of the state the turn's change counts from: the newest state of the
    /// worktree recorded before the turn started, or, for a turn that started while the
    /// state of the turn before was still being recorded, that state, kept with
    /// [`Store::set_turn_start`] once it is recorded. None where there was none, and until
    /// then.
    pub start: Option<String>,
    /// The full id of the state recorded at its end.
    pub state: Option<String>,
}

/// One step of a turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepRecord {
    /// A file read. Kept as its bytes.
    Read(PathBuf),
    /// A file changed with the edit tool, with the lines it changed when the CLI said.
    Edit(PathBuf, Option<Vec<RangeInclusive<u32>>>),
    /// A file written whole.
    Write(PathBuf),
    /// A shell command.
    Command(String),
}

impl Store {
    /// Keeps a new session; refused when the id is taken, and when its CLI's name for it is
    /// taken under the same Adapter.
    pub fn put_session(&self, session: &SessionRecord) -> Result<(), StoreError> {
        // SQLite's integers are signed.
        let ended = session
            .ended
            .map(i64::try_from)
            .transpose()
            .map_err(|_| refusal(format!("session {} ended too late to count", session.id)))?;
        let session = session.clone();
        self.try_ask(move |connection| {
            let keeping = connection.transaction()?;
            if exists(
                &keeping,
                "SELECT 1 FROM session WHERE id = ?1",
                &session.id.0,
            )? {
                return refused(format!("session {} is kept already", session.id));
            }
            if keeping
                .prepare("SELECT 1 FROM session WHERE agent = ?1 AND cli_session = ?2")?
                .exists([&session.agent, &session.cli_session])?
            {
                return refused(format!(
                    "the session {} of {} is kept already",
                    session.cli_session, session.agent
                ));
            }
            keeping.execute(
                "INSERT INTO session
                     (id, workspace, agent, cli_session, terminal, number, running, ended, ended_by)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    session.id.0,
                    session.workspace.0,
                    session.agent,
                    session.cli_session,
                    session.terminal,
                    session.number,
                    session.running,
                    ended,
                    session.ended_by.map(ended_by_number),
                ],
            )?;
            Ok(keeping.commit()?)
        })
    }

    /// The session by its id.
    pub fn session(&self, session: &SessionId) -> Result<Option<SessionRecord>, StoreError> {
        let session = session.0.clone();
        self.ask(move |connection| {
            connection
                .query_row(
                    &format!("{SESSION} WHERE id = ?1"),
                    [session],
                    session_of_row,
                )
                .optional()
        })
    }

    /// The session of the Adapter `agent` that its CLI names `cli_session`, in any
    /// workspace: the one a CLI that resumes it comes back to.
    pub fn session_of_cli(
        &self,
        agent: &str,
        cli_session: &str,
    ) -> Result<Option<SessionRecord>, StoreError> {
        let (agent, cli_session) = (agent.to_owned(), cli_session.to_owned());
        self.ask(move |connection| {
            connection
                .query_row(
                    &format!("{SESSION} WHERE agent = ?1 AND cli_session = ?2"),
                    [agent, cli_session],
                    session_of_row,
                )
                .optional()
        })
    }

    /// Every session of a workspace, in the order they were first kept.
    pub fn sessions_of(&self, workspace: &WorkspaceId) -> Result<Vec<SessionRecord>, StoreError> {
        let workspace = workspace.0.clone();
        self.ask(move |connection| {
            connection
                .prepare(&format!("{SESSION} WHERE workspace = ?1 ORDER BY seq"))?
                .query_map([workspace], session_of_row)?
                .collect()
        })
    }

    /// Every session the Store holds that runs, in every workspace: those a Keeper started
    /// again finds still marked running, which it marks ended by
    /// [`EndedBy::KeeperStopped`].
    pub fn running_sessions(&self) -> Result<Vec<SessionRecord>, StoreError> {
        self.ask(|connection| {
            connection
                .prepare(&format!("{SESSION} WHERE running = 1 ORDER BY seq"))?
                .query_map([], session_of_row)?
                .collect()
        })
    }

    /// Marks a session running in `terminal`, as when its CLI resumed it; how it last
    /// ended is cleared.
    pub fn set_running(
        &self,
        session: &SessionId,
        terminal: Option<&str>,
    ) -> Result<(), StoreError> {
        let (session, terminal) = (session.0.clone(), terminal.map(str::to_owned));
        self.try_ask(move |connection| {
            let changed = connection.execute(
                "UPDATE session
                 SET running = 1, terminal = ?2, ended = NULL, ended_by = NULL
                 WHERE id = ?1",
                params![session, terminal],
            )?;
            if changed == 0 {
                return refused(format!("session {session} is not kept"));
            }
            Ok(())
        })
    }

    /// Marks a session ended, and how; the end gets the next number of the order of ends.
    pub fn set_ended(&self, session: &SessionId, how: EndedBy) -> Result<(), StoreError> {
        let session = session.0.clone();
        self.try_ask(move |connection| {
            // The largest number and the update are one transaction: no end can take the
            // number between them.
            let ending = connection.transaction()?;
            let changed = ending.execute(
                "UPDATE session
                 SET running = 0, ended_by = ?2,
                     ended = (SELECT coalesce(max(ended), 0) + 1 FROM session)
                 WHERE id = ?1",
                params![session, ended_by_number(how)],
            )?;
            if changed == 0 {
                return refused(format!("session {session} is not kept"));
            }
            Ok(ending.commit()?)
        })
    }

    /// Gives a review to a session, or to no agent: the takeover of a review. Refused for a
    /// review or a session the Store does not hold, and for giving a review back to no
    /// agent while the workspace has a review of no agent already.
    pub fn set_review_agent(
        &self,
        review: &ReviewId,
        session: Option<&SessionId>,
    ) -> Result<(), StoreError> {
        let (review, session) = (review.0.clone(), session.map(|s| s.0.clone()));
        self.try_ask(move |connection| {
            let giving = connection.transaction()?;
            let workspace: Option<String> = giving
                .query_row(
                    "SELECT workspace FROM review WHERE id = ?1",
                    [&review],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(workspace) = workspace else {
                return refused(format!("review {review} is not kept"));
            };
            match &session {
                Some(session) => {
                    if !exists(&giving, "SELECT 1 FROM session WHERE id = ?1", session)? {
                        return refused(format!("session {session} is not kept"));
                    }
                }
                None => {
                    let other = giving
                        .prepare(
                            "SELECT 1 FROM review
                             WHERE workspace = ?1 AND agent IS NULL AND id != ?2",
                        )?
                        .exists([&workspace, &review])?;
                    if other {
                        return refused(format!(
                            "workspace {workspace} has a review of no agent already"
                        ));
                    }
                }
            }
            giving.execute(
                "UPDATE review SET agent = ?2 WHERE id = ?1",
                params![review, session],
            )?;
            Ok(giving.commit()?)
        })
    }

    /// Keeps a turn that started; refused when the id is taken, when the session is not
    /// kept, and when the session has a turn of that number already.
    pub fn put_turn(&self, turn: &TurnRecord) -> Result<(), StoreError> {
        let turn = turn.clone();
        self.try_ask(move |connection| {
            let keeping = connection.transaction()?;
            if exists(&keeping, "SELECT 1 FROM turn WHERE id = ?1", &turn.id.0)? {
                return refused(format!("turn {} is kept already", turn.id));
            }
            if !exists(
                &keeping,
                "SELECT 1 FROM session WHERE id = ?1",
                &turn.session.0,
            )? {
                return refused(format!("session {} is not kept", turn.session));
            }
            if keeping
                .prepare("SELECT 1 FROM turn WHERE session = ?1 AND number = ?2")?
                .exists(params![turn.session.0, turn.number])?
            {
                return refused(format!(
                    "session {} has a turn {} already",
                    turn.session, turn.number
                ));
            }
            keeping.execute(
                "INSERT INTO turn (id, session, number, ended, start, state)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    turn.id.0,
                    turn.session.0,
                    turn.number,
                    turn.ended.map(turn_end_number),
                    turn.start,
                    turn.state
                ],
            )?;
            Ok(keeping.commit()?)
        })
    }

    /// Keeps the state a kept turn's change counts from, in the place of the one it had:
    /// for a turn that started while the state of the turn before was still being recorded.
    /// Refused for a turn that is not kept.
    pub fn set_turn_start(&self, turn: &TurnId, start: &str) -> Result<(), StoreError> {
        let (turn, start) = (turn.0.clone(), start.to_owned());
        self.try_ask(move |connection| {
            let changed = connection.execute(
                "UPDATE turn SET start = ?2 WHERE id = ?1",
                params![turn, start],
            )?;
            if changed == 0 {
                return refused(format!("turn {turn} is not kept"));
            }
            Ok(())
        })
    }

    /// Keeps how a turn ended and the state recorded then.
    pub fn end_turn(
        &self,
        turn: &TurnId,
        ended: TurnEndRecord,
        state: Option<&str>,
    ) -> Result<(), StoreError> {
        let (turn, state) = (turn.0.clone(), state.map(str::to_owned));
        self.try_ask(move |connection| {
            let changed = connection.execute(
                "UPDATE turn SET ended = ?2, state = ?3 WHERE id = ?1",
                params![turn, turn_end_number(ended), state],
            )?;
            if changed == 0 {
                return refused(format!("turn {turn} is not kept"));
            }
            Ok(())
        })
    }

    /// Keeps step `number` of a turn; the numbers of a turn go up by one from 1. Refused
    /// for a turn that is not kept, a number below 1 and a number the turn has a step of
    /// already.
    pub fn put_step(
        &self,
        turn: &TurnId,
        number: u32,
        step: &StepRecord,
    ) -> Result<(), StoreError> {
        if number < 1 {
            return Err(refusal(format!(
                "step {number} of turn {turn} cannot be kept: steps count from 1"
            )));
        }
        let (turn, step) = (turn.0.clone(), step.clone());
        let (kind, what, lines) = match &step {
            StepRecord::Read(file) => (0, bytes(file), None),
            StepRecord::Edit(file, lines) => (1, bytes(file), lines.as_deref().map(lines_text)),
            StepRecord::Write(file) => (2, bytes(file), None),
            StepRecord::Command(command) => (3, command.clone().into_bytes(), None),
        };
        self.try_ask(move |connection| {
            let keeping = connection.transaction()?;
            if !exists(&keeping, "SELECT 1 FROM turn WHERE id = ?1", &turn)? {
                return refused(format!("turn {turn} is not kept"));
            }
            if keeping
                .prepare("SELECT 1 FROM step WHERE turn = ?1 AND number = ?2")?
                .exists(params![turn, number])?
            {
                return refused(format!("turn {turn} has a step {number} already"));
            }
            keeping.execute(
                "INSERT INTO step (turn, number, kind, what, lines) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![turn, number, kind, what, lines],
            )?;
            Ok(keeping.commit()?)
        })
    }

    /// A turn with its steps, in their order.
    pub fn turn(&self, turn: &TurnId) -> Result<Option<(TurnRecord, Vec<StepRecord>)>, StoreError> {
        let turn = turn.0.clone();
        self.ask(move |connection| {
            let Some(head) = connection
                .query_row(&format!("{TURN} WHERE turn.id = ?1"), [&turn], turn_of_row)
                .optional()?
            else {
                return Ok(None);
            };
            let steps = connection
                .prepare("SELECT kind, what, lines FROM step WHERE turn = ?1 ORDER BY number")?
                .query_map([&turn], step_of_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(Some((head, steps)))
        })
    }

    /// Every turn of the sessions of a workspace, in the order they started.
    pub fn turns_of(&self, workspace: &WorkspaceId) -> Result<Vec<TurnRecord>, StoreError> {
        let workspace = workspace.0.clone();
        self.ask(move |connection| {
            connection
                .prepare(&format!(
                    "{TURN} JOIN session ON session.id = turn.session
                     WHERE session.workspace = ?1 ORDER BY turn.seq"
                ))?
                .query_map([workspace], turn_of_row)?
                .collect()
        })
    }
}

/// The columns of a session, in the order [`session_of_row`] reads them.
const SESSION: &str = "SELECT id, workspace, agent, cli_session, terminal, number, running,
                              ended, ended_by FROM session";

/// The columns of a turn, in the order [`turn_of_row`] reads them.
const TURN: &str =
    "SELECT turn.id, turn.session, turn.number, turn.ended, turn.start, turn.state FROM turn";

fn session_of_row(row: &Row<'_>) -> rusqlite::Result<SessionRecord> {
    let ended: Option<i64> = row.get(7)?;
    Ok(SessionRecord {
        id: SessionId(row.get(0)?),
        workspace: WorkspaceId(row.get(1)?),
        agent: row.get(2)?,
        cli_session: row.get(3)?,
        terminal: row.get(4)?,
        number: row.get(5)?,
        running: row.get(6)?,
        ended: ended.map(|n| n.unsigned_abs()),
        ended_by: row.get::<_, Option<i64>>(8)?.map(ended_by_of),
    })
}

fn turn_of_row(row: &Row<'_>) -> rusqlite::Result<TurnRecord> {
    Ok(TurnRecord {
        id: TurnId(row.get(0)?),
        session: SessionId(row.get(1)?),
        number: row.get(2)?,
        ended: row.get::<_, Option<i64>>(3)?.map(turn_end_of),
        start: row.get(4)?,
        state: row.get(5)?,
    })
}

fn step_of_row(row: &Row<'_>) -> rusqlite::Result<StepRecord> {
    let (kind, what, lines): (i64, Vec<u8>, Option<String>) =
        (row.get(0)?, row.get(1)?, row.get(2)?);
    Ok(match kind {
        0 => StepRecord::Read(path(what)),
        1 => StepRecord::Edit(path(what), lines.as_deref().map(lines_of)),
        2 => StepRecord::Write(path(what)),
        // The column holds what `put_step` wrote, which was text.
        _ => StepRecord::Command(String::from_utf8_lossy(&what).into_owned()),
    })
}

/// The lines of an edit as kept: `3-3,10-12`, empty for none.
fn lines_text(lines: &[RangeInclusive<u32>]) -> String {
    lines
        .iter()
        .map(|range| format!("{}-{}", range.start(), range.end()))
        .collect::<Vec<_>>()
        .join(",")
}

/// The lines of an edit from the text [`lines_text`] made. A part that is not `a-b` is
/// left out: the column holds nothing else but what the Store wrote.
fn lines_of(text: &str) -> Vec<RangeInclusive<u32>> {
    text.split(',')
        .filter_map(|part| {
            let (first, last) = part.split_once('-')?;
            Some(first.parse().ok()?..=last.parse().ok()?)
        })
        .collect()
}

fn ended_by_number(how: EndedBy) -> i64 {
    match how {
        EndedBy::Cli => 0,
        EndedBy::ProcessGone => 1,
        EndedBy::KeeperStopped => 2,
    }
}

fn ended_by_of(number: i64) -> EndedBy {
    match number {
        0 => EndedBy::Cli,
        1 => EndedBy::ProcessGone,
        _ => EndedBy::KeeperStopped,
    }
}

fn turn_end_number(ended: TurnEndRecord) -> i64 {
    match ended {
        TurnEndRecord::Finished => 0,
        TurnEndRecord::Interrupted => 1,
    }
}

fn turn_end_of(number: i64) -> TurnEndRecord {
    match number {
        0 => TurnEndRecord::Finished,
        _ => TurnEndRecord::Interrupted,
    }
}

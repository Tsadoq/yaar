//! Every changing call is one transaction that waits for the drive: a step is a few bytes,
//! and an agent takes only a few steps a second.

use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use rusqlite::{OptionalExtension, Row, params};

use crate::reviews::{exists, refusal, refused};
use crate::{ReviewId, Store, StoreError, WorkspaceId, bytes, path};

/// 32 lowercase hexadecimal digits the Keeper drew.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(pub(crate) String);

impl SessionId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TurnId(pub(crate) String);

impl TurnId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TurnId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    pub id: SessionId,
    pub workspace: WorkspaceId,
    /// The name of the Adapter of its CLI, such as `claude`.
    pub agent: String,
    pub cli_session: String,
    pub terminal: Option<String>,
    pub number: u32,
    pub running: bool,
    /// The order of ends: a later end has a larger number. None while it runs and for one that
    /// never ended.
    pub ended: Option<u64>,
    pub ended_by: Option<EndedBy>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndedBy {
    Cli,
    ProcessGone,
    /// The Keeper stopped while it ran, as when the machine restarted: cut off, not ended; the app
    /// may offer to resume it.
    KeeperStopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnEndRecord {
    Finished,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRecord {
    pub id: TurnId,
    pub session: SessionId,
    pub number: u32,
    pub ended: Option<TurnEndRecord>,
    /// Full id of the state the turn's change counts from: the newest state recorded before it
    /// started, or, when the previous turn's state was still being recorded then, that state,
    /// kept later by [`Store::set_turn_start`].
    pub start: Option<String>,
    pub state: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepRecord {
    Read(PathBuf),
    Edit(PathBuf, Option<Vec<RangeInclusive<u32>>>),
    Write(PathBuf),
    Command(String),
}

impl Store {
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

    pub fn set_session_cli_record(
        &self,
        session: &SessionId,
        cli_record: Option<&Path>,
    ) -> Result<(), StoreError> {
        let (session, cli_record) = (session.0.clone(), cli_record.map(bytes));
        self.try_ask(move |connection| {
            let changed = connection.execute(
                "UPDATE session SET cli_record = ?2 WHERE id = ?1",
                params![session, cli_record],
            )?;
            if changed == 0 {
                return refused(format!("session {session} is not kept"));
            }
            Ok(())
        })
    }

    pub fn session_cli_record(&self, session: &SessionId) -> Result<Option<PathBuf>, StoreError> {
        let session = session.0.clone();
        self.ask(move |connection| {
            let kept: Option<Option<Vec<u8>>> = connection
                .query_row(
                    "SELECT cli_record FROM session WHERE id = ?1",
                    [session],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(kept.flatten().map(path))
        })
    }

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

    pub fn sessions_of(&self, workspace: &WorkspaceId) -> Result<Vec<SessionRecord>, StoreError> {
        let workspace = workspace.0.clone();
        self.ask(move |connection| {
            connection
                .prepare(&format!("{SESSION} WHERE workspace = ?1 ORDER BY seq"))?
                .query_map([workspace], session_of_row)?
                .collect()
        })
    }

    /// In every workspace: a Keeper started again marks those still running as ended by
    /// [`EndedBy::KeeperStopped`].
    pub fn running_sessions(&self) -> Result<Vec<SessionRecord>, StoreError> {
        self.ask(|connection| {
            connection
                .prepare(&format!("{SESSION} WHERE running = 1 ORDER BY seq"))?
                .query_map([], session_of_row)?
                .collect()
        })
    }

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

    pub fn set_ended(&self, session: &SessionId, how: EndedBy) -> Result<(), StoreError> {
        let session = session.0.clone();
        self.try_ask(move |connection| {
            // The largest number and the update are one transaction: no end can take the number
            // between them.
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

    /// Marks cut-off (`EndedBy::KeeperStopped`) sessions in `workspace` as `EndedBy::ProcessGone`
    /// without changing their `ended` order or touching any review. `Some(session)` clears that
    /// session only; `None` clears every cut-off session in `workspace`. Returns the cleared ids.
    pub fn dismiss_resumable(
        &self,
        workspace: &WorkspaceId,
        session: Option<&SessionId>,
    ) -> Result<Vec<SessionId>, StoreError> {
        let workspace = workspace.0.clone();
        let session = session.map(|s| s.0.clone());
        let stopped = ended_by_number(EndedBy::KeeperStopped);
        let gone = ended_by_number(EndedBy::ProcessGone);
        self.try_ask(move |connection| {
            let clearing = connection.transaction()?;
            let ids: Vec<SessionId> = match &session {
                Some(id) => clearing
                    .prepare(
                        "SELECT id FROM session
                         WHERE workspace = ?1 AND id = ?2 AND running = 0 AND ended_by = ?3",
                    )?
                    .query_map(params![workspace, id, stopped], |row| {
                        Ok(SessionId(row.get(0)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?,
                None => clearing
                    .prepare(
                        "SELECT id FROM session
                         WHERE workspace = ?1 AND running = 0 AND ended_by = ?2
                         ORDER BY seq",
                    )?
                    .query_map(params![workspace, stopped], |row| {
                        Ok(SessionId(row.get(0)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?,
            };
            if !ids.is_empty() {
                match &session {
                    Some(id) => {
                        clearing.execute(
                            "UPDATE session SET ended_by = ?4
                             WHERE workspace = ?1 AND id = ?2 AND running = 0 AND ended_by = ?3",
                            params![workspace, id, stopped, gone],
                        )?;
                    }
                    None => {
                        clearing.execute(
                            "UPDATE session SET ended_by = ?3
                             WHERE workspace = ?1 AND running = 0 AND ended_by = ?2",
                            params![workspace, stopped, gone],
                        )?;
                    }
                }
            }
            clearing.commit()?;
            Ok(ids)
        })
    }

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

const SESSION: &str = "SELECT id, workspace, agent, cli_session, terminal, number, running,
                              ended, ended_by FROM session";

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

fn lines_text(lines: &[RangeInclusive<u32>]) -> String {
    lines
        .iter()
        .map(|range| format!("{}-{}", range.start(), range.end()))
        .collect::<Vec<_>>()
        .join(",")
}

/// A part that is not `a-b` is left out: the column holds nothing else but what the Store wrote.
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

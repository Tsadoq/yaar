//! What the Store keeps of agents: their sessions, the turns of each with the state
//! recorded at its end, and the steps of each turn, so that a Window that attaches later,
//! and a Keeper started again, find them.
//!
//! Every call that changes something is one transaction that waits for the drive, as the
//! calls of slice 1 do: a step is a few bytes, and an agent takes a few steps a second.

use std::path::PathBuf;

use crate::{ReviewId, Store, StoreError, WorkspaceId};

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
    /// The full id of the state recorded at its end.
    pub state: Option<String>,
}

/// One step of a turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepRecord {
    /// A file read. Kept as its bytes.
    Read(PathBuf),
    /// A file changed with the edit tool.
    Edit(PathBuf),
    /// A file written whole.
    Write(PathBuf),
    /// A shell command.
    Command(String),
}

impl Store {
    /// Keeps a new session; refused when the id is taken.
    pub fn put_session(&self, session: &SessionRecord) -> Result<(), StoreError> {
        let _ = session;
        todo!("keep a session")
    }

    /// The session by its id.
    pub fn session(&self, session: &SessionId) -> Result<Option<SessionRecord>, StoreError> {
        let _ = session;
        todo!("find a session")
    }

    /// The session of the Adapter `agent` that its CLI names `cli_session`, in any
    /// workspace: the one a CLI that resumes it comes back to.
    pub fn session_of_cli(
        &self,
        agent: &str,
        cli_session: &str,
    ) -> Result<Option<SessionRecord>, StoreError> {
        let _ = (agent, cli_session);
        todo!("find a session by the CLI's name for it")
    }

    /// Every session of a workspace, in the order they were first kept.
    pub fn sessions_of(&self, workspace: &WorkspaceId) -> Result<Vec<SessionRecord>, StoreError> {
        let _ = workspace;
        todo!("the sessions of a workspace")
    }

    /// Every session the Store holds that runs, in every workspace: those a Keeper started
    /// again finds still marked running, which it marks ended.
    pub fn running_sessions(&self) -> Result<Vec<SessionRecord>, StoreError> {
        todo!("the sessions marked running")
    }

    /// Marks a session running in `terminal`, or ended; an end gets the next number of
    /// the order of ends.
    pub fn set_running(
        &self,
        session: &SessionId,
        running: bool,
        terminal: Option<&str>,
    ) -> Result<(), StoreError> {
        let _ = (session, running, terminal);
        todo!("mark a session running or ended")
    }

    /// Gives a review to a session, or to no agent: the takeover of a review.
    pub fn set_review_agent(
        &self,
        review: &ReviewId,
        session: Option<&SessionId>,
    ) -> Result<(), StoreError> {
        let _ = (review, session);
        todo!("give a review to a session")
    }

    /// Keeps a turn that started; refused when the id is taken.
    pub fn put_turn(&self, turn: &TurnRecord) -> Result<(), StoreError> {
        let _ = turn;
        todo!("keep a turn")
    }

    /// Keeps how a turn ended and the state recorded then.
    pub fn end_turn(
        &self,
        turn: &TurnId,
        ended: TurnEndRecord,
        state: Option<&str>,
    ) -> Result<(), StoreError> {
        let _ = (turn, ended, state);
        todo!("keep the end of a turn")
    }

    /// Keeps step `number` of a turn; the numbers of a turn go up by one from 1.
    pub fn put_step(
        &self,
        turn: &TurnId,
        number: u32,
        step: &StepRecord,
    ) -> Result<(), StoreError> {
        let _ = (turn, number, step);
        todo!("keep a step")
    }

    /// A turn with its steps, in their order.
    pub fn turn(&self, turn: &TurnId) -> Result<Option<(TurnRecord, Vec<StepRecord>)>, StoreError> {
        let _ = turn;
        todo!("a turn and its steps")
    }

    /// Every turn of the sessions of a workspace, in the order they started.
    pub fn turns_of(&self, workspace: &WorkspaceId) -> Result<Vec<TurnRecord>, StoreError> {
        let _ = workspace;
        todo!("the turns of a workspace")
    }
}

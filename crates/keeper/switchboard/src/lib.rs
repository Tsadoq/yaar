//! Switchboard (`H-EVENTS, H-SESSION`).
//!
//! Owns: what hooks and extensions report, held while no window is attached; and for each
//! agent session its CLI, terminal, review and state.
//!
//! Surface:
//! - [`Switchboard`], which takes the events the Adapters made of what the CLIs reported,
//!   and what the Keeper saw itself (a process that is gone, a review marked reviewed),
//!   keeps every session with its state and its open turn, and gives what changed as
//!   [`Change`]s, in order;
//! - [`State`], what a session is doing, and [`shown`], the one state a workspace with
//!   several sessions shows;
//! - [`Known`], a session as the Switchboard holds it; [`SessionId`] and [`TurnId`], the
//!   app's names for a session and a turn.
//!
//! The Switchboard decides nothing for the user and answers nothing a CLI asks: it
//! listens. It keeps sessions in memory; the Keeper keeps what must outlive it in the
//! Store, from the changes this gives. It knows neither the Link nor the Store, and no CLI
//! by name: an Adapter's name is a text to it.

use std::fmt;

use yaar_agent::{CliSession, Event, Reported, TurnEnd};

/// What a session is doing.
///
/// The order of the variants is the order in which a workspace with several sessions
/// shows them, the first first: see [`shown`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum State {
    /// The agent waits at a permission prompt for the user's answer.
    WaitsForPermission,
    /// The agent finished a turn that the reviewer has not marked reviewed since.
    Unreviewed,
    /// The agent works on a turn.
    Works,
    /// The agent runs and waits for the user, with nothing unreviewed.
    Idle,
    /// The session ended, or its process is gone.
    NotRunning,
}

/// The state a workspace shows for its sessions: the first of these that holds for any
/// of them, waits for permission, finished a turn and not reviewed, works, idle, not
/// running. None for no session.
pub fn shown(states: impl IntoIterator<Item = State>) -> Option<State> {
    let _ = states;
    todo!("the first state of the order that holds for any session")
}

/// The app's name for one agent session: 32 lowercase hexadecimal digits, drawn once when
/// the session first reported in. A session the CLI resumes keeps it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(pub String);

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The app's name for one turn: 32 lowercase hexadecimal digits, drawn when it started.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TurnId(pub String);

impl fmt::Display for TurnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One session as the Switchboard holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Known {
    /// The app's id of it.
    pub id: SessionId,
    /// The name of the Adapter of its CLI.
    pub agent: String,
    /// The CLI's own name for it.
    pub cli: CliSession,
    /// The terminal it runs in, by the id the app gave it.
    pub terminal: Option<String>,
    /// The process of the CLI; 0 when none was reported.
    pub pid: u32,
    /// What it is doing.
    pub state: State,
    /// Its open turn, with the number of steps told in it so far; none between turns.
    pub turn: Option<(TurnId, u32)>,
}

/// What changed, in the order it happened. The Keeper keeps it and tells the Windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// A session reported in for the first time, or a session that had ended was resumed
    /// by its CLI.
    Started {
        /// The session.
        session: SessionId,
        /// The name of the Adapter of its CLI.
        agent: String,
        /// The CLI's own name for it.
        cli: CliSession,
        /// The terminal it runs in.
        terminal: Option<String>,
        /// Whether it is a session that ran before, resumed.
        resumed: bool,
    },
    /// A session's state is now this.
    State {
        /// The session.
        session: SessionId,
        /// Its state.
        state: State,
    },
    /// A turn started.
    TurnStarted {
        /// The session.
        session: SessionId,
        /// The turn.
        turn: TurnId,
    },
    /// The agent took one more step in its open turn: a file read, a file written, or a
    /// shell command that began.
    Step {
        /// The session.
        session: SessionId,
        /// The turn.
        turn: TurnId,
        /// The number of the step in the turn, from 1.
        number: u32,
        /// The event that was the step: [`Event::FileRead`], [`Event::FileWritten`] or
        /// [`Event::Command`] that began.
        event: Event,
    },
    /// A shell command of the session ended; it is no step, but who wrote what during it
    /// is judged by when it ran.
    CommandEnded {
        /// The session.
        session: SessionId,
    },
    /// The open turn ended.
    TurnEnded {
        /// The session.
        session: SessionId,
        /// The turn.
        turn: TurnId,
        /// How.
        how: TurnEnd,
    },
    /// The session ended, as its CLI reported or as its process went.
    Ended {
        /// The session.
        session: SessionId,
    },
}

/// Every agent session the Keeper has heard of since it started, with its state.
#[derive(Debug, Default)]
pub struct Switchboard {
    _todo: (),
}

impl Switchboard {
    /// A Switchboard that knows no session.
    pub fn new() -> Switchboard {
        todo!("an empty switchboard")
    }

    /// A session the Keeper kept from an earlier run, as it is in the Store: not running.
    /// Its CLI may resume it, and then it keeps its id.
    pub fn remember(&mut self, session: SessionId, agent: &str, cli: CliSession) {
        let _ = (session, agent, cli);
        todo!("a session known from the Store")
    }

    /// Takes one event of the Adapter named `agent`, and gives what it changed, in order.
    /// `draw` gives a new id, for a session or a turn, each time it is called.
    ///
    /// A session first heard of is [`Change::Started`], idle. A turn that starts makes it
    /// work; a permission it waits for, and its answer, turn waiting on and off; any step
    /// after a permission it waited for is the answer; a turn that ends, by the CLI's word
    /// or by the mark of an interrupt, leaves it finished and not reviewed; the session's
    /// end, not running. An event of a session that has ended is taken as its CLI resuming
    /// it. A step outside a turn opens none: it is told with no turn, as nothing.
    pub fn take(
        &mut self,
        agent: &str,
        reported: Reported,
        draw: &mut dyn FnMut() -> String,
    ) -> Vec<Change> {
        let _ = (agent, reported, draw);
        todo!("the changes one event makes")
    }

    /// The process `pid` of a CLI is gone, with no end reported: every session of it is
    /// not running, and its open turn, if any, ends as interrupted.
    pub fn gone(&mut self, pid: u32) -> Vec<Change> {
        let _ = pid;
        todo!("the changes of a process that is gone")
    }

    /// The review of `session` was marked reviewed: a session that finished a turn not
    /// yet reviewed is idle now.
    pub fn reviewed(&mut self, session: &SessionId) -> Vec<Change> {
        let _ = session;
        todo!("the changes of a mark of reviewed")
    }

    /// The session by its id.
    pub fn session(&self, session: &SessionId) -> Option<Known> {
        let _ = session;
        todo!("one session")
    }

    /// Every session, in the order they were first heard of.
    pub fn sessions(&self) -> Vec<Known> {
        todo!("every session")
    }

    /// The processes of the sessions that run: those the Keeper watches for their end.
    pub fn pids(&self) -> Vec<u32> {
        todo!("the processes of the running sessions")
    }
}

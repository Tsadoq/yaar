//! The agents' side of the Keeper: the Adapters of the supported CLIs and their pieces,
//! the Switchboard, the sessions, turns and steps kept in the Store, the review each
//! session takes, the state recorded at each turn's end, and what the Windows ask of them:
//! the agents, a turn, the history of a worktree and the versions of a change.
//!
//! How it fits the rest of the Keeper. A report of a CLI's piece comes in as
//! `Request::Report`, with the moment it arrived, and is put on the queue of this side's
//! own thread before it is answered with `Reply::Done`; the marks the Dock finds in a
//! terminal go on the same queue. That thread works them out in the order they came, so
//! that no hook of a CLI waits for a recording, and so that a mark printed after a hook
//! returned is always taken after that hook's report. Every change of a session is kept in
//! the Store before it is told, and told to every Window under the Keeper's turn, so that
//! a Window that asks for the agents never takes an older list for a newer one.
//!
//! What a Window reads of agents, turns and the history of a worktree is answered in
//! `reading.rs`, from the Store, the state stores and Workspaces, through `Kept`: it holds
//! nothing of this side's thread.
//!
//! Until this side is built, every request it answers is refused with [`NOT_WIRED`] and it
//! gives the terminals nothing: no entries, no marks, no launcher.

mod reading;

use std::sync::{Arc, Weak};

use yaar_link::{AgentReport, Refusal, Reply, ReviewId, TerminalId};

use crate::watching::Watching;
use crate::wiring::{Kept, Outcome};
use crate::{Options, Shared};

/// The reason given for a request this side does not answer yet.
pub(crate) const NOT_WIRED: &str = "the Keeper does not answer this yet";

/// The agents' side of the Keeper. Shared by the threads that answer Windows and by the
/// Dock's threads, which tell it the marks they found and the terminals that ended: every
/// call takes `&self`, and none waits for a recording.
pub(crate) struct Agents {
    _todo: (),
}

impl Agents {
    /// Starts this side: every supported Adapter, each with its pieces written by
    /// `Adapter::install` into `<data folder>/agents/<its name>`, and `options.companions`
    /// as where the pieces find the companions; the Switchboard, told of every session the
    /// Store holds; and those the Store holds as running marked ended by
    /// `EndedBy::KeeperStopped`, since a Keeper that starts runs none and they were cut
    /// off. `kept` is none for a Keeper with no git or no database, which follows no agent.
    /// A workspace is watched, through `watching`, while a session runs in it.
    pub(crate) fn start(
        options: &Options,
        kept: Option<&Kept>,
        watching: Arc<Watching>,
        shared: Weak<Shared>,
    ) -> Agents {
        let _ = (options, kept, watching, shared);
        Agents { _todo: () }
    }

    /// What every shell of the app gets from the Adapters, in the Dock's words.
    pub(crate) fn entries(&self) -> Vec<yaar_dock::Entry> {
        Vec::new()
    }

    /// The runs of bytes every terminal is watched for: the marks of every Adapter, one
    /// after the other in the order of the Adapters. [`Agents::marked`] is told the index
    /// in this list.
    pub(crate) fn marks(&self) -> Vec<Vec<u8>> {
        Vec::new()
    }

    /// What the launcher of the CLI named `agent` runs in the user's shell; none for a
    /// name no Adapter has.
    pub(crate) fn launcher(&self, agent: &str) -> Option<&'static str> {
        let _ = agent;
        None
    }

    /// Takes one report of a CLI's piece: stamps the moment it arrived, puts it on the
    /// queue of this side's thread, and only then answers `Reply::Done`; refused only for
    /// an Adapter the Keeper does not have. Nothing else is done on the caller's thread: a
    /// report the Adapter cannot read is written down by that thread. A report from
    /// outside the app's terminals, or from a terminal the Keeper does not hold, is
    /// answered and followed no further.
    pub(crate) fn report(&self, kept: &Kept, report: AgentReport) -> Outcome {
        let _ = (kept, report);
        Outcome::failed(NOT_WIRED)
    }

    /// The Dock found mark `mark` of [`Agents::marks`] in the output of `terminal`: put on
    /// the same queue as the reports, and nothing else on the Dock's thread.
    pub(crate) fn marked(&self, terminal: &TerminalId, mark: usize) {
        let _ = (terminal, mark);
    }

    /// The program of `terminal` ended: a session that ran in it runs no more.
    pub(crate) fn terminal_ended(&self, terminal: &TerminalId) {
        let _ = terminal;
    }

    /// `Request::ListAgents`: the sessions as the Switchboard holds them.
    pub(crate) fn list(&self, kept: &Kept) -> Result<Reply, Refusal> {
        let _ = kept;
        Err(not_wired())
    }

    /// The review `review` was marked reviewed: the session it belongs to, if any, has
    /// nothing unreviewed any more, and every Window is told.
    pub(crate) fn reviewed(&self, kept: &Kept, review: &ReviewId) {
        let _ = (kept, review);
    }
}

/// The refusal of a request this side does not answer yet.
pub(crate) fn not_wired() -> Refusal {
    Refusal::Failed {
        reason: NOT_WIRED.to_owned(),
    }
}

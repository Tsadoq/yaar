//! The agents' side of the Keeper: the Adapters of the supported CLIs and their pieces,
//! the Switchboard, the sessions, turns and steps kept in the Store, the review each
//! session takes, the state recorded at each turn's end, and what the Windows ask of them:
//! the agents, a turn, the history of a worktree and the versions of a change.
//!
//! How it fits the rest of the Keeper. A report of a CLI's piece comes in as
//! `Request::Report` and is answered at once with `Reply::Done`; what it means is worked
//! out on a thread of this side's own, in the order the reports came, so that no hook of
//! a CLI waits for a recording. Every change of a session is kept in the Store before it
//! is told, and told to every Window under the Keeper's turn, so that a Window that asks
//! for the agents never takes an older list for a newer one.
//!
//! Until this side is built, every request it answers is refused with [`NOT_WIRED`] and it
//! gives the terminals nothing: no entries, no marks, no launcher.

use std::path::Path;
use std::sync::{Arc, Weak};

use yaar_link::{AgentReport, Refusal, Reply, ReviewId, TerminalId, TurnId, WorkspaceId};

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
    /// `Adapter::install` into `<data folder>/agents/<its name>`, and the folder of the
    /// app's programs, the one that holds the Keeper program, as where the pieces find the
    /// companions; the Switchboard, told of every session the Store holds; and those the
    /// Store holds as running marked ended, since a Keeper that starts runs none. `kept`
    /// is none for a Keeper with no git or no database, which follows no agent. A
    /// workspace is watched, through `watching`, while a session runs in it.
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

    /// Takes one report of a CLI's piece: answered `Reply::Done` at once, refused for an
    /// Adapter the Keeper does not have or a report it cannot read. A report from outside
    /// the app's terminals, or from a terminal the Keeper does not hold, is answered and
    /// followed no further.
    pub(crate) fn report(&self, kept: &Kept, report: AgentReport) -> Outcome {
        let _ = (kept, report);
        Outcome::failed(NOT_WIRED)
    }

    /// The Dock found mark `mark` of [`Agents::marks`] in the output of `terminal`.
    pub(crate) fn marked(&self, terminal: &TerminalId, mark: usize) {
        let _ = (terminal, mark);
    }

    /// The program of `terminal` ended: a session that ran in it runs no more.
    pub(crate) fn terminal_ended(&self, terminal: &TerminalId) {
        let _ = terminal;
    }

    /// The review `review` was marked reviewed: the session it belongs to, if any, has
    /// nothing unreviewed any more, and every Window is told.
    pub(crate) fn reviewed(&self, kept: &Kept, review: &ReviewId) {
        let _ = (kept, review);
    }

    /// `Request::ListAgents`.
    pub(crate) fn list(&self, kept: &Kept) -> Result<Reply, Refusal> {
        let _ = kept;
        Err(not_wired())
    }

    /// `Request::GetTurn`.
    pub(crate) fn turn(&self, kept: &Kept, turn: &TurnId) -> Result<Reply, Refusal> {
        let _ = (kept, turn);
        Err(not_wired())
    }

    /// `Request::GetHistory`.
    pub(crate) fn history(&self, kept: &Kept, workspace: &WorkspaceId) -> Result<Reply, Refusal> {
        let _ = (kept, workspace);
        Err(not_wired())
    }

    /// `Request::CompareVersions`.
    pub(crate) fn compare(
        &self,
        kept: &Kept,
        workspace: &WorkspaceId,
        from: &str,
        to: Option<&str>,
    ) -> Result<Reply, Refusal> {
        let _ = (kept, workspace, from, to);
        Err(not_wired())
    }

    /// `Request::ReadVersions`.
    pub(crate) fn read_versions(
        &self,
        kept: &Kept,
        workspace: &WorkspaceId,
        old: Option<(&str, &Path)>,
        new: (Option<&str>, &Path),
    ) -> Result<Reply, Refusal> {
        let _ = (kept, workspace, old, new);
        Err(not_wired())
    }
}

/// The refusal of a request this side does not answer yet.
pub(crate) fn not_wired() -> Refusal {
    Refusal::Failed {
        reason: NOT_WIRED.to_owned(),
    }
}

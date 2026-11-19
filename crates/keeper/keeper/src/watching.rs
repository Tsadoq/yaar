//! Which worktrees the Keeper watches, and what it tells of them: those a Window shows,
//! and those an agent runs in.
//!
//! The watcher of a worktree is Workspaces'. Here it is started and stopped, and what it
//! sees is told to every Window: a change of files as `Fact::ChangesChanged` of the
//! workspace, a change of what is checked out as that and `Fact::Workspaces` with the list
//! as it is then. While files keep changing, as when an agent writes, a workspace's fact
//! is told once at the start and then at most once in each [`CALM`], after the change, so
//! that a Window reading the changes is not made to start again at every save.
//!
//! Until this part is built, `Request::ShowWorkspace` is refused with
//! [`crate::agents::NOT_WIRED`] and nothing is watched.

use std::sync::{Arc, Weak};
use std::time::Duration;

use yaar_link::{Refusal, Reply, WorkspaceId};

use crate::agents::not_wired;
use crate::wiring::Kept;
use crate::{Shared, Window};

/// The shortest time between two facts of one workspace that its watcher brings.
pub(crate) const CALM: Duration = Duration::from_millis(250);

/// The worktrees watched, and why each is.
pub(crate) struct Watching {
    _todo: (),
}

impl Watching {
    /// Watches nothing yet; tells what it sees to the Windows of `shared`.
    pub(crate) fn new(shared: Weak<Shared>) -> Watching {
        let _ = shared;
        Watching { _todo: () }
    }

    /// `window` shows `workspace` now, or none: answers `Request::ShowWorkspace`. The
    /// workspace's watcher is started, on a thread of its own, when nothing watched it;
    /// the one `window` showed before is let go when nothing else needs it.
    pub(crate) fn show(
        &self,
        kept: &Kept,
        window: &Arc<Window>,
        workspace: Option<&WorkspaceId>,
    ) -> Result<Reply, Refusal> {
        let _ = (kept, window, workspace);
        Err(not_wired())
    }

    /// `window` has gone: what it showed is let go when nothing else needs it.
    pub(crate) fn window_gone(&self, kept: &Kept, window: &Arc<Window>) {
        let _ = (kept, window);
    }

    /// An agent runs in `workspace` now, or no longer does: the agents' side says so, and
    /// the workspace is watched while one runs, whether a Window shows it or not.
    pub(crate) fn agent_runs(&self, kept: &Kept, workspace: &WorkspaceId, runs: bool) {
        let _ = (kept, workspace, runs);
    }
}

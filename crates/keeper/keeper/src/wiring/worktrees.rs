//! `Request::CreateWorktree`: a new worktree made from a branch, and an agent started in it,
//! in one step (FR-014).
//!
//! In this order, each step only once the one before succeeded:
//!
//! 1. The agent: an Adapter by the name `agent` is among the Keeper's (`Agents::supported`);
//!    else refused with `Refusal::Failed` "there is no agent CLI named <agent>", and nothing
//!    is made.
//! 2. The worktree: `Workspaces::create_worktree(repository, from, branch)`, which places it
//!    beside the repository, makes it with the user's own git, opens it and sets its base to
//!    `from`. Its refusals map to the Link's: `UnknownRepository` to `Refusal::Failed` "there
//!    is no repository <id>"; `UnknownRevision` to `Refusal::UnknownRevision`;
//!    `NotABranchName`, `BranchExists` and `FolderExists` to `Refusal::Failed` with the
//!    refusal's own sentence. Nothing is made.
//! 3. The terminal: the agent's launcher (`Program::Agent { agent }`) opened in the new
//!    workspace at `size`, as `OpenTerminal` opens it (`Parts::open_terminal`), so that the
//!    CLI runs as when the user types its name in a shell of the app. When it cannot be
//!    opened, `Workspaces::remove_created` takes the worktree and its branch away again and
//!    the refusal of the opening is given: nothing half made is left.
//! 4. The reply, `Reply::WorktreeCreated { workspace, terminal }`, with the step that lets
//!    the terminal's output through once the reply is queued, as for `OpenTerminal`; and
//!    `Fact::Workspaces` with the list as it is then, after the reply.
//!
//! It waits its turn, as `OpenFolder` does: it changes the list of workspaces. It is answered
//! on the connection's thread, as `OpenTerminal` is; git's `worktree add` takes as long as a
//! checkout of the branch takes, and the Window shows that the worktree is being made.

use std::sync::Arc;

use yaar_link::{Fact, Refusal, Reply, RepositoryId, TerminalSize};

use crate::Window;
use crate::wiring::{Kept, Parts, Step};

impl Parts {
    /// `Request::CreateWorktree`, as the module doc says: the reply and the step that lets
    /// the agent's output through, or the refusal; puts `Fact::Workspaces` into `facts`.
    #[expect(
        clippy::too_many_arguments,
        reason = "the fields of the request, as the Link carries them"
    )]
    pub(super) fn create_worktree(
        &self,
        kept: &Kept,
        window: &Arc<Window>,
        repository: &RepositoryId,
        from: &str,
        branch: &str,
        agent: &str,
        size: TerminalSize,
        facts: &mut Vec<Fact>,
    ) -> Result<(Reply, Step), Refusal> {
        let _ = (kept, window, repository, from, branch, agent, size, facts);
        todo!("check the agent, make the worktree, open the launcher, as the module doc says")
    }
}

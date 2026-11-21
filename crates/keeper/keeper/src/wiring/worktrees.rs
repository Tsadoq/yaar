//! `Request::CreateWorktree`: a new worktree made from a branch, and an agent started in it,
//! in one step (FR-014).
//!
//! The user's own git checks a whole branch out and runs the user's hooks (a
//! `post-checkout` that installs packages can take a minute), so it runs with nothing of
//! the Keeper held: the request is answered apart and does not wait its turn
//! (`wiring::answerer`, `wiring::waits_its_turn`), and the git runs in the work
//! [`Parts::create_worktree`] gives back as `Answered::Later`, after the parts are let go.
//! In this order, each step only once the one before succeeded:
//!
//! 1. Under the parts, at once: the agent, an Adapter by the name `agent` among the Keeper's
//!    (`Agents::supported`), else refused with `Refusal::Failed` "there is no agent CLI named
//!    <agent>"; then the folder the user named for the repository's worktrees, read from the
//!    Store (`Store::settings`, the `yaar_store::Setting::WorktreesFolder` of `repository`;
//!    none when it is not listed or its folder is none), handed over as `folder` to
//!    `Workspaces::plan_worktree(repository, from, branch, folder)`, whose refusal is given
//!    at once. Nothing is made. Otherwise the work of step 2 is given back.
//! 2. The work, with the parts let go: `Planned::add`, the user's git. Its refusals and its
//!    failure map to the Link's as `wiring::refusal` maps a `WorkspaceError`:
//!    `UnknownRepository`, `NotABranchName`, `BranchExists`, `FolderExists`,
//!    `FolderNotAbsolute` and `FolderInsideRepository` to `Refusal::Failed` with the
//!    refusal's own sentence, `NoSuchFolder` to `Refusal::NoSuchFolder`, `UnknownRevision` to
//!    `Refusal::UnknownRevision`, git's failure to `Refusal::Failed` with git's words.
//!    Nothing is left made (the Recorder takes away what a failing git left).
//! 3. Then, still in the work: the Keeper's shared state is reached again through the
//!    `Weak<Shared>` the parts hold (the field `shared` of `Parts`, cloned into the work at
//!    step 1), its turn is taken (the field `turn` of `Shared`, with `crate::lock`, as `answer`
//!    takes it for a request that waits its turn), and its parts are read again. When the Keeper is leaving (the `Weak` is dead,
//!    or the parts are gone), `Planned::take_away` removes what step 2 made and the
//!    refusal is `Refusal::Failed` "the Keeper is leaving". Otherwise, under the turn:
//!    `Workspaces::open_added`, which opens the worktree and sets its base to `from`; then
//!    the agent's launcher (`Program::Agent { agent }`) opened in the new workspace at
//!    `size`, as `OpenTerminal` opens it (`Parts::open_terminal`), so that the CLI runs as
//!    when the user types its name in a shell of the app. When the launcher cannot be
//!    opened, `Workspaces::remove_created` takes the worktree and its branch away again and
//!    the refusal of the opening is given: nothing half made is left. When it opens,
//!    `Fact::Workspaces` with the list as it is then is told to every Window
//!    (`Shared::tell`), still under the turn, so that it takes its place among the other
//!    lists told; it reaches the asking Window before the reply.
//! 4. The turn let go, the work gives the outcome: `Reply::WorktreeCreated { workspace,
//!    terminal }`, no further fact, and the step that lets the terminal's output through
//!    once the reply is queued, as for `OpenTerminal`.
//!
//! The Window shows that the worktree is being made until the reply comes
//! (`yaar_terminal_pane::WORKTREE_BEING_MADE`); nothing bounds the time of the user's hooks.
//!
//! # The folder the user names for a repository's worktrees
//!
//! `Request::SetSetting` of `Setting::WorktreesFolder` is [`set_worktrees_folder`]'s (the
//! settings module hands it over): under the Keeper's turn, as every setting is kept,
//! 1. a repository the Store does not hold is refused with `Refusal::Failed` "the
//!    repository <id> is not known", as for the answer on its language servers;
//! 2. a folder (not none) is checked by `Workspaces::check_worktrees_folder(repository,
//!    folder)`, whose refusal is mapped as step 2 above maps one; none is never refused:
//!    it is the way back to the rule of `yaar_workspaces`' `create` module;
//! 3. then kept, `Store::set_setting(&yaar_store::Setting::WorktreesFolder { .. })` (a
//!    refusal of the Store is `Refusal::Failed` with its sentence), answered `Reply::Done`,
//!    and `Fact::SettingChanged` with the Link's setting as it came is put into `facts`.
//!
//! A refusal keeps nothing and tells nothing. What is checked only when a worktree is made
//! (the folder's parent is there, nothing but a folder is at it, no worktree of the
//! repository has come to lie around it since) is `plan_worktree`'s.

use std::path::Path;
use std::sync::Arc;

use yaar_link::{Fact, Refusal, Reply, RepositoryId, TerminalSize};

use crate::Window;
use crate::wiring::{Answered, Kept, Parts};

impl Parts {
    /// `Request::CreateWorktree`, as the module doc says: the refusal at once, or the work
    /// that makes the worktree and opens the agent's launcher.
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
    ) -> Answered {
        let _ = (kept, window, repository, from, branch, agent, size);
        todo!("check the agent, plan, then the work of the module doc")
    }
}

/// `Request::SetSetting` of `Setting::WorktreesFolder`: checks and keeps the folder the
/// user named for the worktrees of `repository`, or `None` for the rule the app follows
/// without one, as the module doc says; `Reply::Done`, and `Fact::SettingChanged` in
/// `facts`, once the Store holds it.
pub(crate) fn set_worktrees_folder(
    kept: &Kept,
    repository: &RepositoryId,
    folder: Option<&Path>,
    facts: &mut Vec<Fact>,
) -> Result<Reply, Refusal> {
    let _ = (kept, repository, folder, facts);
    todo!("check the folder, keep it and tell it, as the module doc says")
}

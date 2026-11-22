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
//!    or the parts are gone; the work's thread keeps the shared state alive, so in practice
//!    only the parts are), `Planned::take_away` removes what step 2 made and the
//!    refusal is `Refusal::Failed` "the Keeper is leaving". Otherwise, under the turn:
//!    `Workspaces::open_added`, which opens the worktree and sets its base to `from`; then
//!    the agent's launcher (`Program::Agent { agent }`) opened in the new workspace at
//!    `size`, as `OpenTerminal` opens it (`Parts::open_terminal`), so that the CLI runs as
//!    when the user types its name in a shell of the app. When the launcher cannot be
//!    opened, `Workspaces::remove_created` takes the worktree and its branch away again and
//!    the refusal of the opening is given: nothing half made is left. When even that fails
//!    (the user's own hook can lock the new worktree), the sentence says which worktree
//!    stays, the Keeper's error output has it too, and the list as it is then, which holds
//!    the worktree, is told as below. When it opens,
//!    `Fact::Workspaces` with the list as it is then is told to every Window
//!    (`Shared::tell`), still under the turn, so that it takes its place among the other
//!    lists told; it reaches the asking Window before the reply. A list that cannot be
//!    read is not told and is noted on the Keeper's error output.
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

use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, Weak};

use yaar_link::{Fact, Program, Refusal, Reply, RepositoryId, TerminalSize};
use yaar_workspaces::Planned;

use crate::wiring::{Answered, Kept, Outcome, Parts, list, refusal};
use crate::{Shared, Window, lock};

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
        // The agent first: a CLI the app has no Adapter for is no reason to run git.
        if !self.agents.supported().iter().any(|cli| cli.agent == agent) {
            return Answered::Now(Outcome::failed(format!(
                "there is no agent CLI named {agent}"
            )));
        }
        let repository = yaar_workspaces::RepositoryId::new(repository.as_str());
        // Read here, under the parts, and not in the work: the setting is as it was when
        // the user asked.
        let folder = match named_folder(kept, &repository) {
            Ok(folder) => folder,
            Err(reason) => return Answered::Now(Outcome::failed(reason)),
        };
        let planned =
            match kept
                .workspaces
                .plan_worktree(&repository, from, branch, folder.as_deref())
            {
                Ok(planned) => planned,
                Err(error) => return Answered::Now(refused(refusal(error))),
            };
        // The work owns everything it uses: it runs after the parts are let go.
        let (shared, window, agent) = (self.shared.clone(), Arc::clone(window), agent.to_owned());
        Answered::Later(Box::new(move || {
            make(&planned, &shared, &window, agent, size)
        }))
    }
}

/// Steps 2 to 4 of the module doc.
fn make(
    planned: &Planned,
    shared: &Weak<Shared>,
    window: &Arc<Window>,
    agent: String,
    size: TerminalSize,
) -> Outcome {
    // Step 2: the user's own git, with nothing of the Keeper held.
    let added = match planned.add() {
        Ok(added) => added,
        Err(error) => return refused(refusal(error)),
    };
    // Step 3: the Keeper's turn, then its parts again, which are gone when it is leaving.
    let leaving = || {
        if let Err(error) = planned.take_away(&added) {
            // Nobody is left to tell, and a worktree stays in the user's repository.
            eprintln!(
                "yaar-keeper: the worktree at {} could not be taken away: {error}",
                planned.folder().display()
            );
        }
        Outcome::failed("the Keeper is leaving")
    };
    let Some(shared) = shared.upgrade() else {
        return leaving();
    };
    let _turn = lock(&shared.turn);
    let held = shared.parts.read().unwrap_or_else(PoisonError::into_inner);
    let Some(parts) = held.as_ref() else {
        return leaving();
    };
    let Some(kept) = parts.kept() else {
        return leaving();
    };
    let created = match kept.workspaces.open_added(planned, added) {
        Ok(created) => created,
        // Taken away again by `open_added`.
        Err(error) => return refused(refusal(error)),
    };
    let workspace = yaar_link::WorkspaceId::new(created.workspace.as_str());
    let opened = parts.open_terminal(kept, window, &workspace, &Program::Agent { agent }, size);
    let (terminal, release) = match opened {
        Ok((Reply::TerminalOpened { terminal }, release)) => (terminal, release),
        Ok(_) => unreachable!("opening a terminal is answered by the terminal"),
        Err(mut refused_by_the_dock) => {
            // Nothing half made is left: the worktree and its branch go again. When they
            // cannot, the user is told that a worktree stays.
            if let Err(error) = kept.workspaces.remove_created(&created) {
                let left = format!(
                    "the worktree at {} could not be taken away: {error}",
                    planned.folder().display()
                );
                eprintln!("yaar-keeper: {left}");
                if let Refusal::Failed { reason } = &mut refused_by_the_dock {
                    *reason = format!("{reason} ({left})");
                }
                // The worktree stays in git's list and in the Store, so that the next
                // list a Window asks for holds it: the Windows are told the same.
                tell_the_list(&shared, kept);
            }
            return refused(refused_by_the_dock);
        }
    };
    // Told under the turn, so that the list takes its place among the others told, and
    // before the reply, which is queued once this returns.
    tell_the_list(&shared, kept);
    Outcome {
        reply: Ok(Reply::WorktreeCreated {
            workspace,
            terminal,
        }),
        facts: Vec::new(),
        then: Some(release),
    }
}

/// Tells every Window the list of workspaces as it is now; a list that cannot be read is
/// not told and is noted on the Keeper's error output. Called under the Keeper's turn.
fn tell_the_list(shared: &Shared, kept: &Kept) {
    match list(&kept.workspaces) {
        Ok(repositories) => shared.tell(&[Fact::Workspaces(repositories)]),
        Err(error) => eprintln!("yaar-keeper: the list of workspaces could not be told: {error:?}"),
    }
}

/// An outcome that is a refusal and tells nothing.
fn refused(refusal: Refusal) -> Outcome {
    Outcome {
        reply: Err(refusal),
        facts: Vec::new(),
        then: None,
    }
}

/// The folder the user named for the worktrees of `repository`: none when no setting names
/// one or the setting is none. The sentence of the failure when the Store cannot be read.
fn named_folder(
    kept: &Kept,
    repository: &yaar_workspaces::RepositoryId,
) -> Result<Option<PathBuf>, String> {
    let settings = kept.store.settings().map_err(|error| error.to_string())?;
    Ok(settings.into_iter().find_map(|setting| match setting {
        yaar_store::Setting::WorktreesFolder {
            repository: of,
            folder,
        } if of == *repository => folder,
        _ => None,
    }))
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
    let failed = |error: &dyn std::fmt::Display| Refusal::Failed {
        reason: error.to_string(),
    };
    let held = yaar_workspaces::RepositoryId::new(repository.as_str());
    let known = kept.store.repositories().map_err(|e| failed(&e))?;
    if !known.iter().any(|known| known.id == held) {
        return Err(Refusal::Failed {
            reason: format!("the repository {repository} is not known"),
        });
    }
    if let Some(folder) = folder {
        kept.workspaces
            .check_worktrees_folder(&held, folder)
            .map_err(refusal)?;
    }
    kept.store
        .set_setting(&yaar_store::Setting::WorktreesFolder {
            repository: held,
            folder: folder.map(Path::to_owned),
        })
        .map_err(|e| failed(&e))?;
    facts.push(Fact::SettingChanged(yaar_link::Setting::WorktreesFolder {
        repository: repository.clone(),
        folder: folder.map(Path::to_owned),
    }));
    Ok(Reply::Done)
}

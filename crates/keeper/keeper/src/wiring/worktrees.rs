//! `Request::CreateWorktree`: the user's git runs their hooks (a `post-checkout` can take a
//! minute), so the work runs after the parts are let go, as `Answered::Later`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, Weak};

use yaar_link::{Fact, Program, Refusal, Reply, RepositoryId, TerminalSize};
use yaar_workspaces::Planned;

use crate::wiring::{Answered, Kept, Outcome, Parts, list, refusal};
use crate::{Shared, Window, lock};

impl Parts {
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
        if !self.agents.supported().iter().any(|cli| cli.agent == agent) {
            return Answered::Now(Outcome::failed(format!(
                "there is no agent CLI named {agent}"
            )));
        }
        let repository = yaar_workspaces::RepositoryId::new(repository.as_str());
        // Read here, under the parts: the setting is as it was when the user asked.
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
        let (shared, window, agent) = (self.shared.clone(), Arc::clone(window), agent.to_owned());
        Answered::Later(Box::new(move || {
            make(&planned, &shared, &window, agent, size)
        }))
    }
}

fn make(
    planned: &Planned,
    shared: &Weak<Shared>,
    window: &Arc<Window>,
    agent: String,
    size: TerminalSize,
) -> Outcome {
    let added = match planned.add() {
        Ok(added) => added,
        Err(error) => return refused(refusal(error)),
    };
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
        Ok(_) => {
            // Not reached: opening a terminal answers with a terminal. What the Dock did is not
            // known, so the worktree is left and the user is told which one stays.
            let left = format!(
                "the worktree at {} stays: opening the launcher gave no terminal",
                planned.folder().display()
            );
            eprintln!("yaar-keeper: {left}");
            tell_the_list(&shared, kept);
            return refused(Refusal::Failed { reason: left });
        }
        Err(refused_by_the_dock) => {
            let Err(error) = kept.workspaces.remove_created(&created) else {
                return refused(refused_by_the_dock);
            };
            let left = format!(
                "the worktree at {} could not be taken away: {error}",
                planned.folder().display()
            );
            eprintln!("yaar-keeper: {left}");
            // The worktree stays in git's list and in the Store, so the Windows are told the list
            // that holds it.
            tell_the_list(&shared, kept);
            return refused(with_the_worktree_that_stays(refused_by_the_dock, &left));
        }
    };
    // The list is told under the turn, so it takes its place among the others told, and before the
    // reply, which is queued once this returns.
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

fn tell_the_list(shared: &Shared, kept: &Kept) {
    match list(&kept.workspaces) {
        Ok(repositories) => shared.tell(&[Fact::Workspaces(repositories)]),
        Err(error) => eprintln!("yaar-keeper: the list of workspaces could not be told: {error:?}"),
    }
}

fn with_the_worktree_that_stays(refusal: Refusal, left: &str) -> Refusal {
    let reason = match refusal {
        Refusal::Failed { reason } => reason,
        other => other.to_string(),
    };
    Refusal::Failed {
        reason: format!("{reason} ({left})"),
    }
}

fn refused(refusal: Refusal) -> Outcome {
    Outcome {
        reply: Err(refusal),
        facts: Vec::new(),
        then: None,
    }
}

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

/// A `folder` of `None` is never refused: it goes back to the rule the app follows without a named
/// folder.
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

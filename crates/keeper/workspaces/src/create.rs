//! A worktree the app creates, from a branch, for an agent to work in (FR-014, FR-004).
//!
//! [`Workspaces::create_worktree`] makes a new worktree of an opened repository, on a new
//! branch made from the branch `from`, with the user's own git
//! (`yaar_recorder::Worktree::add_worktree`), opens it as a workspace, as
//! [`Workspaces::open_folder`] opens any folder, and sets its base to `from`, as
//! [`Workspaces::set_base`] does: the base of a worktree the app created is the branch it
//! was made from, kept by its short name and followed as every branch base is.
//!
//! # Where the worktree lies
//!
//! Beside the repository, where the repository keeps its worktrees: the first of these that
//! applies gives the folder the new worktree is made in, and the worktree is that folder
//! joined with the branch's name in which every `/` is made `-` (`feature/login` gives
//! `feature-login`).
//!
//! 1. The repository has linked worktrees, besides its main one, whose folders all lie in one
//!    folder: that folder. A bare repository's worktrees are all linked.
//! 2. Otherwise a folder beside the main worktree, named after it with `.worktrees` added:
//!    `/src/app` gives `/src/app.worktrees`. For a bare repository with no worktree in a shared
//!    folder, beside its git folder: `/src/app.git` gives `/src/app.git.worktrees`.
//!
//! The folders are read from `git worktree list --porcelain` (`yaar_recorder::Worktree::list`),
//! a worktree whose folder is gone left out; "lies in" is the parent folder of each, compared
//! as git prints them. The folder of rule 2 is made when it is not there.
//!
//! # What is refused, and what is left
//!
//! | Case | Refusal | Checked |
//! | --- | --- | --- |
//! | The Store holds no repository with this id | [`Refusal::UnknownRepository`] | first |
//! | The repository's every worktree is gone or cannot be read | [`Refusal::UnknownRepository`] | then |
//! | `branch` is no name git takes for a branch | [`Refusal::NotABranchName`] | by the Recorder, before git runs |
//! | A branch named `branch` exists already, checked out or not | [`Refusal::BranchExists`] | by the Recorder, before git runs |
//! | `from` names no branch, local or of a remote | [`Refusal::UnknownRevision`] | by the Recorder, before git runs |
//! | Something is at the folder the rules give | [`Refusal::FolderExists`] | by the Recorder, before git runs |
//!
//! A refusal leaves the repository, its worktrees and branches, and the Store as they were.
//! When git fails after its checks, git removes what it made itself. When opening the new
//! worktree or keeping its base fails after git made it, the worktree and its branch are
//! taken away again (`yaar_recorder::Worktree::remove_added`) before the error is given, so
//! that nothing half made is left. [`Workspaces::remove_created`] does the same for a caller
//! whose own next step failed, as the Keeper's when the agent cannot be started; the Store's
//! record of the workspace then stays, as that of any worktree that is gone, and is not
//! listed (it holds no review).
//!
//! The id file is written into the new worktree's git folder by the opening, as for every
//! worktree; nothing else of the app is written into the worktree or the repository.

use std::path::PathBuf;

use yaar_recorder::Added;

use crate::{Base, RepositoryId, WorkspaceError, WorkspaceId, Workspaces};

/// A worktree [`Workspaces::create_worktree`] made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    /// The repository it belongs to.
    pub repository: RepositoryId,
    /// Its workspace.
    pub workspace: WorkspaceId,
    /// Its folder.
    pub folder: PathBuf,
    /// Its base: `from`, as the user set it.
    pub base: Base,
    /// What git made, for [`Workspaces::remove_created`].
    pub added: Added,
}

impl Workspaces {
    /// Makes a new worktree of `repository` on a new branch `branch` made from the branch
    /// `from`, opens it as a workspace and sets its base to `from`, as the module doc says.
    pub fn create_worktree(
        &self,
        repository: &RepositoryId,
        from: &str,
        branch: &str,
    ) -> Result<Created, WorkspaceError> {
        let _ = (repository, from, branch);
        todo!("place, add, open and set the base, as the module doc says")
    }

    /// Takes away a worktree [`Workspaces::create_worktree`] made, and its branch, as the
    /// module doc says.
    pub fn remove_created(&self, created: &Created) -> Result<(), WorkspaceError> {
        let _ = created;
        todo!("remove the worktree and its branch, as the module doc says")
    }
}

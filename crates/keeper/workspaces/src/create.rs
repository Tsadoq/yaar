//! A worktree the app creates, from a branch, for an agent to work in (FR-014, FR-004).
//!
//! Three steps, so that the caller can run the long one, the user's own git, without
//! holding anything else up (the Keeper runs it after it let go of its parts):
//!
//! 1. [`Workspaces::plan_worktree`]: the repository and the folder, read from the Store and
//!    from git; nothing is written. Gives a [`Planned`], which owns what the next step
//!    needs.
//! 2. [`Planned::add`]: the user's own git makes the worktree on a new branch made from the
//!    branch `from` (`yaar_recorder::Worktree::add_worktree`, which checks first and leaves
//!    nothing when it refuses or fails). Needs no `Workspaces`.
//! 3. [`Workspaces::open_added`]: opens the new worktree as a workspace, as
//!    [`Workspaces::open_folder`] opens any folder, and sets its base to `from`, as
//!    [`Workspaces::set_base`] does: the base of a worktree the app created is the branch it
//!    was made from, kept by its short name and followed as every branch base is. When
//!    either fails, the worktree and its branch are taken away again
//!    (`yaar_recorder::Worktree::remove_added`) before the error is given.
//!
//! A caller whose own step after 2 fails takes the worktree away with [`Planned::take_away`]
//! (before step 3) or [`Workspaces::remove_created`] (after it), so that nothing half made is
//! left; the Store's record of a workspace removed after step 3 then stays, as that of any
//! worktree that is gone, and is not listed (it holds no review).
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
//! as git prints them. The folder of rule 2 is made by step 2, after its checks passed, when
//! it is not there.
//!
//! # What is refused, and what is left
//!
//! | Case | Refusal | Checked |
//! | --- | --- | --- |
//! | The Store holds no repository with this id | [`Refusal::UnknownRepository`] | by step 1 |
//! | The repository's every worktree is gone or cannot be read | [`Refusal::UnknownRepository`] | by step 1 |
//! | `branch` is no name git takes for a branch | [`Refusal::NotABranchName`] | by step 2, before git runs |
//! | A branch named `branch` exists already, checked out or not | [`Refusal::BranchExists`] | by step 2, before git runs |
//! | `from` names no branch, local or of a remote | [`Refusal::UnknownRevision`] | by step 2, before git runs |
//! | Something is at the folder the rules give | [`Refusal::FolderExists`] | by step 2, before git runs |
//! | git fails | `WorkspaceError::Git`, with git's words | by step 2 |
//!
//! A refusal leaves the repository, its worktrees, branches, configuration and objects, the
//! folder beside it, and the Store as they were.
//!
//! The id file is written into the new worktree's git folder by step 3, as for every
//! worktree; nothing else of the app is written into the worktree or the repository.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use yaar_recorder::{Added, Worktree};

use crate::{Base, RepositoryId, WorkspaceError, WorkspaceId, Workspaces};

/// What [`Workspaces::plan_worktree`] found: everything [`Planned::add`] needs, owned, so
/// that it can run on any thread, after the caller let go of the parts.
#[derive(Debug, Clone)]
pub struct Planned {
    repository: RepositoryId,
    // A worktree of the repository, through which the user's git runs.
    worktree: Arc<Worktree>,
    folder: PathBuf,
    branch: String,
    from: String,
}

impl Planned {
    /// The folder the new worktree is made in, as the module doc's rules give it.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Step 2 of the module doc: the user's own git makes the worktree; its refusals map to
    /// [`Refusal`](crate::Refusal) as the module doc's table says, a failure of git to
    /// `WorkspaceError::Git`.
    pub fn add(&self) -> Result<Added, WorkspaceError> {
        let _ = (&self.repository, &self.worktree, &self.branch, &self.from);
        todo!("add the worktree with the user's git, as the module doc says")
    }

    /// Takes away a worktree [`Planned::add`] made and nothing opened yet, and its branch,
    /// as `yaar_recorder::Worktree::remove_added` says.
    pub fn take_away(&self, added: &Added) -> Result<(), WorkspaceError> {
        let _ = added;
        todo!("remove what the add made, as the module doc says")
    }
}

/// A worktree made and opened as a workspace.
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
    // What git made, for `Workspaces::remove_created`.
    added: Added,
}

impl Workspaces {
    /// Step 1 of the module doc: the repository `repository` and the folder a new worktree
    /// on the branch `branch`, made from the branch `from`, would lie in. Writes nothing.
    pub fn plan_worktree(
        &self,
        repository: &RepositoryId,
        from: &str,
        branch: &str,
    ) -> Result<Planned, WorkspaceError> {
        let _ = (repository, from, branch);
        todo!("find the repository and place the folder, as the module doc says")
    }

    /// Step 3 of the module doc: opens the worktree `added` of `planned` as a workspace and
    /// sets its base to the branch it was made from; takes it away again when either fails.
    pub fn open_added(&self, planned: &Planned, added: Added) -> Result<Created, WorkspaceError> {
        let _ = (planned, added);
        todo!("open and set the base, as the module doc says")
    }

    /// Takes away a worktree [`Workspaces::open_added`] opened, and its branch, as the
    /// module doc says.
    pub fn remove_created(&self, created: &Created) -> Result<(), WorkspaceError> {
        let _ = &created.added;
        todo!("remove the worktree and its branch, as the module doc says")
    }
}

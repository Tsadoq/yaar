//! A worktree the app creates, from a branch, for an agent to work in (FR-014, FR-004).
//!
//! Three steps, so that the caller can run the long one, the user's own git, without
//! holding anything else up (the Keeper runs it after it let go of its parts):
//!
//! 1. [`Workspaces::plan_worktree`]: the repository and the folder, read from the Store and
//!    from git, in the folder the user named for the repository when the caller hands one
//!    (the Keeper reads it from the Store's settings); nothing is written. Gives a
//!    [`Planned`], which owns what the next step needs.
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
//! In the folder the user named for the repository, or else beside the repository, where
//! the repository keeps its worktrees: the first of these that applies gives the folder the
//! new worktree is made in, and the worktree is that folder joined with the branch's name in
//! which every `/` is made `-` (`feature/login` gives `feature-login`).
//!
//! 0. The caller hands a folder (`folder` of [`Workspaces::plan_worktree`]: the user's
//!    `WorktreesFolder` setting of the repository): that folder, as given, once
//!    [`Workspaces::check_worktrees_folder`] passed it again and the checks of creation
//!    below passed. Rules 1 and 2 are then not read.
//! 1. The repository has linked worktrees, besides its main one, whose folders all lie in one
//!    folder: that folder. A bare repository's worktrees are all linked.
//! 2. Otherwise a folder beside the main worktree, named after it with `.worktrees` added:
//!    `/src/app` gives `/src/app.worktrees`. For a bare repository with no worktree in a shared
//!    folder, beside its git folder: `/src/app.git` gives `/src/app.git.worktrees`.
//!
//! The folders are read from `git worktree list --porcelain` (`yaar_recorder::Worktree::list`),
//! a worktree whose folder is gone left out; "lies in" is the parent folder of each, compared
//! as git prints them. The folder of rule 2, or of rule 0, is made by step 2, after its
//! checks passed, when it is not there; only that one folder is made, never its parents.
//!
//! # A folder the user names
//!
//! [`Workspaces::check_worktrees_folder`] says whether a folder may hold the worktrees the
//! app creates for a repository. The Keeper runs it when the user sets the folder, before
//! the Store keeps it, and step 1 runs it again at each creation, since a worktree can have
//! come to lie around the folder since. It writes nothing and refuses:
//!
//! | Case | Refusal |
//! | --- | --- |
//! | The Store holds no repository with this id, or none of its worktrees can be read | [`Refusal::UnknownRepository`] |
//! | The folder is not absolute, or one of its parts is `..` | [`Refusal::FolderNotAbsolute`] |
//! | The folder is, or lies inside, the folder of a worktree of the repository (the main one and every linked one `git worktree list --porcelain` lists whose folder is there) or the repository's git folder (`git rev-parse --git-common-dir`, made absolute) | [`Refusal::FolderInsideRepository`] |
//!
//! "Is, or lies inside" compares whole parts of paths (`Path::starts_with`), each side
//! resolved: a worktree's folder and the git folder by `fs::canonicalize`; the named folder
//! by `fs::canonicalize` of the longest leading part of it that exists, with the parts after
//! it appended as written. So a folder reached through a symbolic link into a worktree, or
//! one not made yet below a worktree, is refused. A folder beside the repository, its
//! parent, or any other folder outside them, is taken.
//!
//! Checked only when a worktree is made, by step 1, after the checks above and before
//! anything is made, since the folder may be made, mounted or moved between the setting and
//! the creation:
//!
//! | Case | Refusal |
//! | --- | --- |
//! | Something that is not a folder is at the named folder (a symbolic link to a folder counts as the folder) | [`Refusal::NoSuchFolder`] of the named folder |
//! | Nothing is at the named folder and nothing is at its parent folder either | [`Refusal::NoSuchFolder`] of the parent |
//!
//! A folder the user named that is not there is made by step 2, as rule 2's is, and taken
//! away again with the worktree when the creation fails (`Added::made_parent`). Whether
//! the user may write into it is git's to find: its failure is `WorkspaceError::Git`, and
//! nothing is left.
//!
//! # What is refused, and what is left
//!
//! | Case | Refusal | Checked |
//! | --- | --- | --- |
//! | The Store holds no repository with this id | [`Refusal::UnknownRepository`] | by step 1 |
//! | The repository's every worktree is gone or cannot be read | [`Refusal::UnknownRepository`] | by step 1 |
//! | The caller's `folder` is refused, as "A folder the user names" says | [`Refusal::FolderNotAbsolute`], [`Refusal::FolderInsideRepository`], [`Refusal::NoSuchFolder`] | by step 1 |
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

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use yaar_recorder::{AddError, Added, Listed, Worktree};

use crate::{Base, Refusal, RepositoryId, WorkspaceError, WorkspaceId, Workspaces, lock};

/// What [`Workspaces::plan_worktree`] found: everything [`Planned::add`] needs, owned, so
/// that it can run on any thread, after the caller let go of the parts.
#[derive(Debug, Clone)]
pub struct Planned {
    repository: RepositoryId,
    // A worktree of the repository, through which the user's git runs.
    worktree: Arc<Worktree>,
    // The folder of the new worktree itself: the folder it is made in, joined with the
    // branch's name.
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
        self.worktree
            .add_worktree(&self.folder, &self.branch, &self.from)
            .map_err(|e| match e {
                AddError::NotABranchName(name) => Refusal::NotABranchName(name).into(),
                AddError::BranchExists(name) => Refusal::BranchExists(name).into(),
                AddError::UnknownBranch(name) => Refusal::UnknownRevision(name).into(),
                AddError::FolderExists(folder) => Refusal::FolderExists(folder).into(),
                AddError::Git(e) => WorkspaceError::Git(e),
            })
    }

    /// Takes away a worktree [`Planned::add`] made and nothing opened yet, and its branch,
    /// as `yaar_recorder::Worktree::remove_added` says.
    pub fn take_away(&self, added: &Added) -> Result<(), WorkspaceError> {
        self.worktree.remove_added(added).map_err(Into::into)
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

/// A repository as it is now: a worktree to run git through, and every worktree it lists.
struct Found {
    worktree: Arc<Worktree>,
    listed: Vec<Listed>,
}

impl Workspaces {
    /// Step 1 of the module doc: the repository `repository` and the folder a new worktree
    /// on the branch `branch`, made from the branch `from`, would lie in: in `folder` when
    /// the caller hands the one the user named for the repository, else by the rules of
    /// the module doc. Writes nothing.
    pub fn plan_worktree(
        &self,
        repository: &RepositoryId,
        from: &str,
        branch: &str,
        folder: Option<&Path>,
    ) -> Result<Planned, WorkspaceError> {
        let found = self.find(repository, None)?;
        let made_in = match folder {
            Some(named) => {
                refuse_folder(&found, named)?;
                refuse_unless_there(named)?;
                named.to_owned()
            }
            None => shared_folder(&found),
        };
        Ok(Planned {
            repository: repository.clone(),
            worktree: found.worktree,
            folder: made_in.join(branch.replace('/', "-")),
            branch: branch.to_owned(),
            from: from.to_owned(),
        })
    }

    /// Whether `folder` may hold the worktrees the app creates for `repository`, as the
    /// module doc's "A folder the user names" says: `Ok` when it may, else the refusal.
    /// Writes nothing.
    pub fn check_worktrees_folder(
        &self,
        repository: &RepositoryId,
        folder: &Path,
    ) -> Result<(), WorkspaceError> {
        let found = self.find(repository, None)?;
        refuse_folder(&found, folder).map_err(Into::into)
    }

    /// Step 3 of the module doc: opens the worktree `added` of `planned` as a workspace and
    /// sets its base to the branch it was made from; takes it away again when either fails.
    pub fn open_added(&self, planned: &Planned, added: Added) -> Result<Created, WorkspaceError> {
        let opened = match self.open_folder(&added.folder) {
            Ok(opened) => opened,
            Err(e) => {
                let _ = planned.take_away(&added);
                return Err(e);
            }
        };
        let base = match self.set_base(&opened.workspace, &planned.from) {
            Ok(base) => base,
            Err(e) => {
                // Opened, so that the Store holds it and a running git is kept for it:
                // both are let go before the folder is taken away.
                lock(&self.worktrees).remove(&opened.workspace);
                let _ = planned.take_away(&added);
                return Err(e);
            }
        };
        Ok(Created {
            repository: planned.repository.clone(),
            workspace: opened.workspace,
            folder: added.folder.clone(),
            base,
            added,
        })
    }

    /// Takes away a worktree [`Workspaces::open_added`] opened, and its branch, as the
    /// module doc says.
    pub fn remove_created(&self, created: &Created) -> Result<(), WorkspaceError> {
        // Through another worktree of the repository than the one that goes.
        let found = self.find(&created.repository, Some(&created.folder))?;
        self.unwatch(&created.workspace);
        lock(&self.worktrees).remove(&created.workspace);
        found.worktree.remove_added(&created.added)?;
        Ok(())
    }

    /// The repository `repository` as git lists its worktrees now, and one of them,
    /// other than `leaving`, to run git through.
    fn find(
        &self,
        repository: &RepositoryId,
        leaving: Option<&Path>,
    ) -> Result<Found, WorkspaceError> {
        let record = self
            .store
            .repositories()?
            .into_iter()
            .find(|record| &record.id == repository)
            .ok_or_else(|| Refusal::UnknownRepository(repository.clone()))?;
        let listed = Worktree::list_of(&self.git, &record.git_dir)?;
        let leaving = leaving.map(resolved);
        let worktree = listed
            .iter()
            .filter(|one| leaving.as_deref() != Some(one.path.as_path()))
            // One that cannot be opened, for whatever reason, is left to the next.
            .find_map(|one| Worktree::open(&self.git, &one.path).ok().map(Arc::new))
            .ok_or_else(|| Refusal::UnknownRepository(repository.clone()))?;
        Ok(Found { worktree, listed })
    }
}

/// Rules 1 and 2 of the module doc: where the repository keeps its worktrees.
fn shared_folder(found: &Found) -> PathBuf {
    let mut parents = found
        .listed
        .iter()
        .filter(|one| !one.main)
        .map(|one| one.path.parent());
    if let Some(Some(first)) = parents.next()
        && parents.all(|parent| parent == Some(first))
    {
        return first.to_owned();
    }
    // The main worktree, else (a bare repository) the git folder.
    let beside = found
        .listed
        .iter()
        .find(|one| one.main)
        .map_or_else(|| found.worktree.common_dir(), |main| &main.path);
    let mut name = beside.file_name().unwrap_or_default().to_owned();
    name.push(".worktrees");
    beside.with_file_name(name)
}

/// The refusals of a folder named for the worktrees of a repository that do not depend on
/// what is at it: the ones made when it is set and again when a worktree is made.
fn refuse_folder(found: &Found, folder: &Path) -> Result<(), Refusal> {
    if !folder.is_absolute() || folder.components().any(|part| part == Component::ParentDir) {
        return Err(Refusal::FolderNotAbsolute(folder.to_owned()));
    }
    let wanted = resolved(folder);
    let lies_in = found
        .listed
        .iter()
        .map(|one| one.path.as_path())
        .chain([found.worktree.common_dir()])
        .any(|worktree| wanted.starts_with(worktree));
    if lies_in {
        return Err(Refusal::FolderInsideRepository(folder.to_owned()));
    }
    Ok(())
}

/// What a worktree made in the named `folder` needs of it: a folder, or a place to make one
/// in. Asked only when a worktree is made.
fn refuse_unless_there(folder: &Path) -> Result<(), Refusal> {
    match fs::metadata(folder) {
        Ok(found) if found.is_dir() => return Ok(()),
        Ok(_) => return Err(Refusal::NoSuchFolder(folder.to_owned())),
        Err(_) => {}
    }
    // Something that cannot be followed, a link to nothing, is something that is not a
    // folder; otherwise nothing is there, and its parent folder has to be.
    if fs::symlink_metadata(folder).is_ok() {
        return Err(Refusal::NoSuchFolder(folder.to_owned()));
    }
    match folder.parent() {
        Some(parent) if fs::metadata(parent).is_ok_and(|found| found.is_dir()) => Ok(()),
        Some(parent) => Err(Refusal::NoSuchFolder(parent.to_owned())),
        None => Err(Refusal::NoSuchFolder(folder.to_owned())),
    }
}

/// `path` with its links resolved as far as it exists, and what is below that as written.
fn resolved(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut at = path;
    loop {
        if let Ok(found) = fs::canonicalize(at) {
            return rest
                .iter()
                .rev()
                .fold(found, |whole, part| whole.join(part));
        }
        match (at.parent(), at.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name);
                at = parent;
            }
            _ => return path.to_owned(),
        }
    }
}

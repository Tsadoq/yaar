//! ! Three steps, so that the slow one (git) runs after the Keeper let go of its parts:
//! ! `plan_worktree`, `Planned::add`, `open_added`. The folder is the caller's, else the one all
//! ! linked worktrees share, else `<main worktree>.worktrees` (bare: `<git folder>.worktrees`).

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use yaar_recorder::{AddError, Added, Listed, Worktree};

use crate::{Base, Refusal, RepositoryId, WorkspaceError, WorkspaceId, Workspaces, lock};

#[derive(Debug, Clone)]
pub struct Planned {
    repository: RepositoryId,
    worktree: Arc<Worktree>,
    folder: PathBuf,
    branch: String,
    from: String,
}

impl Planned {
    pub fn folder(&self) -> &Path {
        &self.folder
    }

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

    pub fn take_away(&self, added: &Added) -> Result<(), WorkspaceError> {
        self.worktree.remove_added(added).map_err(Into::into)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    pub repository: RepositoryId,
    pub workspace: WorkspaceId,
    pub folder: PathBuf,
    pub base: Base,
    added: Added,
}

struct Found {
    worktree: Arc<Worktree>,
    listed: Vec<Listed>,
}

impl Workspaces {
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

    pub fn check_worktrees_folder(
        &self,
        repository: &RepositoryId,
        folder: &Path,
    ) -> Result<(), WorkspaceError> {
        let found = self.find(repository, None)?;
        refuse_folder(&found, folder).map_err(Into::into)
    }

    /// Takes the worktree away again when opening or recording the base fails.
    pub fn open_added(&self, planned: &Planned, added: Added) -> Result<Created, WorkspaceError> {
        let opened = match self.open_folder(&added.folder) {
            Ok(opened) => opened,
            Err(e) => {
                take_away_noting(planned, &added);
                return Err(e);
            }
        };
        let base = match self.set_base(&opened.workspace, &planned.from) {
            Ok(base) => base,
            Err(e) => {
                // Drop the cached worktree, and with it the running git, before the folder goes.
                lock(&self.worktrees).remove(&opened.workspace);
                take_away_noting(planned, &added);
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

    /// The workspace is forgotten only once the worktree is gone: if git cannot remove it
    /// (a hook can lock it) the error is returned and the workspace stays listed and watched.
    pub fn remove_created(&self, created: &Created) -> Result<(), WorkspaceError> {
        let found = self.find(&created.repository, Some(&created.folder))?;
        found.worktree.remove_added(&created.added)?;
        self.unwatch(&created.workspace);
        lock(&self.worktrees).remove(&created.workspace);
        Ok(())
    }

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
            .find_map(|one| Worktree::open(&self.git, &one.path).ok().map(Arc::new))
            .ok_or_else(|| Refusal::UnknownRepository(repository.clone()))?;
        Ok(Found { worktree, listed })
    }
}

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

/// The caller gets the opening's error; a worktree that stays is noted on stderr.
fn take_away_noting(planned: &Planned, added: &Added) {
    if let Err(error) = planned.take_away(added) {
        eprintln!(
            "yaar-keeper: the worktree at {} could not be taken away: {error}",
            added.folder.display()
        );
    }
}

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

/// Links resolved as far as the path exists, the rest appended as written, so a folder not
/// made yet below a worktree is still caught.
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

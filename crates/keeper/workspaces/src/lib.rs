//! Workspaces (`H-WORKSPACE`).
//!
//! Owns: workspaces, worktree identity, base, creating worktrees, the worktree watcher, who is credited with a changed path.
//!
//! Surface:
//! - [`Workspaces`]: opening a folder, the repositories with their worktrees, those that
//!   are gone and hold reviews among them, the base of a workspace and setting it, the
//!   changes of a workspace and the two versions of one of its files, and the worktree of
//!   a workspace for the parts that read, record and commit in it;
//! - [`default_base`] and [`on_base_branch`], the two rules behind the base, as plain
//!   functions of what git reports;
//! - what those return: [`Repository`], [`Workspace`], [`Base`], [`Changes`],
//!   [`Comparison`] with [`NoComparison`], [`FileSides`], and [`WorkspaceError`] with the
//!   [`Refusal`] a user can mend.
//!
//! A workspace is one worktree. It is recognised by an id written once into the git
//! folder of that worktree, in a file named [`ID_FILE`]: the id stays with the worktree
//! when the folder is moved, and git lists nothing in that place. Everything else the app
//! keeps about a workspace is in the Store, under that id.
//!
//! A folder that is copied takes the id file along. [`Workspaces::open_folder`] says what
//! becomes of an id found in two places.
//!
//! This crate starts no git and opens no database: it asks the Recorder and the Store.

mod base;
mod identity;

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use yaar_recorder::{Branch, Commits, Git, GitError, OpenError};
use yaar_store::{RepositoryRecord, Store, StoreError, WorkspaceRecord};

pub use base::{default_base, on_base_branch};
pub use yaar_recorder::{Change, ChangedPath, Commit, CommitId, Head, Refs, Worktree};
pub use yaar_store::{RepositoryId, WorkspaceId};

/// The name of the file, in the git folder of a worktree, that holds the id of its
/// workspace: 32 lowercase hexadecimal digits and a line feed.
pub const ID_FILE: &str = "ade-id";

/// A repository the user opened, with its worktrees as git lists them now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    /// Its id.
    pub id: RepositoryId,
    /// The name to show: the name of the folder that holds the repository.
    pub name: String,
    /// The git folder all its worktrees share.
    pub git_dir: PathBuf,
    /// Its worktrees, the main one first, then in git's order.
    pub workspaces: Vec<Workspace>,
}

/// One worktree of an opened repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    /// Its id.
    pub id: WorkspaceId,
    /// The folder of the worktree: where it was last seen, when it is gone.
    pub path: PathBuf,
    /// What is checked out in it; none when the worktree is gone. See
    /// [`Workspaces::repositories`] for when a worktree that is gone is listed.
    pub head: Option<Head>,
}

/// What opening a folder gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    /// The repository the folder lies in.
    pub repository: RepositoryId,
    /// The workspace of the worktree the folder lies in.
    pub workspace: WorkspaceId,
}

/// What the changes of a workspace are counted from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Base {
    /// A branch by its short name, as [`Refs`] lists it, or a commit by its full id.
    pub name: String,
    /// Who chose it.
    pub source: BaseSource,
}

/// Who chose the base of a workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseSource {
    /// Nobody: [`default_base`] found it.
    Default,
    /// The user set it.
    User,
}

/// What the changes of a workspace are compared with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Comparison {
    /// The branch against its base, from the point where it left the base.
    SinceBase {
        /// The base.
        base: Base,
    },
    /// On its base branch, which follows an upstream.
    NotOnUpstream {
        /// The base, as [`Workspaces::base`] returns it. The branch checked out is that
        /// branch, or the local branch of the same name when the base is a branch of a
        /// remote: `main` against `origin/main`.
        base: Base,
        /// The branch that the branch checked out follows, by its short name.
        upstream: String,
    },
    /// On its base branch, which follows nothing, in a repository with a remote.
    OnNoRemoteBranch {
        /// The base, as [`Workspaces::base`] returns it; the branch checked out is that
        /// branch or its local twin, as in [`Comparison::NotOnUpstream`].
        base: Base,
    },
    /// Nothing to compare with. Whoever shows this says why and offers to set a base.
    NothingToCompare {
        /// The base, when there is one.
        base: Option<Base>,
        /// Why there is nothing to compare with.
        why: NoComparison,
    },
}

/// Why there is nothing to compare a workspace with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoComparison {
    /// The workspace is on its base branch, in a repository with no remote.
    NoRemote,
    /// [`default_base`] found no base and the user set none.
    NoBase,
    /// The base the user set names no branch and no commit any more.
    BaseGone,
    /// The branch and its base share no commit, or nothing is committed on the branch
    /// yet.
    NoSharedCommit,
}

/// The changes of one workspace.
///
/// | [`Comparison`] | `counted_from` | `commits` |
/// | --- | --- | --- |
/// | `SinceBase` | the point where the branch left the base | of the branch since that point |
/// | `NotOnUpstream` | the commit checked out | not on the upstream |
/// | `OnNoRemoteBranch` | the commit checked out | on no branch of any remote |
/// | `NothingToCompare` | the commit checked out, if any | none |
///
/// `files` are the paths that differ between `counted_from` and the files on disk, which
/// takes in what is not committed and what git does not track yet. With nothing committed
/// every file on disk that git does not ignore is new.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changes {
    /// What the workspace is compared with.
    pub comparison: Comparison,
    /// The commit the files are counted from; none only where nothing is committed yet.
    /// [`Workspaces::file`] reads the old version of a file in it.
    pub counted_from: Option<CommitId>,
    /// The commits of the comparison, newest first.
    pub commits: Vec<Commit>,
    /// The changed files, ordered by path.
    pub files: Vec<ChangedPath>,
}

/// The two versions of one file of a workspace. A version is none where the file does not
/// exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSides {
    /// The file in the commit that was named, at the path it had there.
    pub old: Option<Vec<u8>>,
    /// The file on disk.
    pub new: Option<Vec<u8>>,
}

/// Why a request was refused, in a way the user can mend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// There is no folder at this path, or it cannot be read.
    NoSuchFolder(PathBuf),
    /// The folder lies in no git repository and no worktree.
    NotARepository(PathBuf),
    /// The folder is a repository with no files checked out.
    BareRepository(PathBuf),
    /// The folder is a copy of a worktree that shares the git folder of the original.
    CopiedWorktree(PathBuf),
    /// No workspace has this id.
    UnknownWorkspace(WorkspaceId),
    /// git knows no branch and no commit by this name.
    UnknownRevision(String),
    /// The path leads out of the worktree.
    OutsideWorkspace(PathBuf),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchFolder(path) => write!(f, "there is no folder at {}", path.display()),
            Self::NotARepository(path) => write!(
                f,
                "{} holds no git repository and no worktree",
                path.display()
            ),
            Self::BareRepository(path) => write!(
                f,
                "{} is a repository with no files checked out",
                path.display()
            ),
            Self::CopiedWorktree(path) => write!(
                f,
                "{} is a copy of a worktree and shares its git folder",
                path.display()
            ),
            Self::UnknownWorkspace(id) => write!(f, "there is no workspace {id}"),
            Self::UnknownRevision(name) => {
                write!(f, "git knows no branch and no commit named {name}")
            }
            Self::OutsideWorkspace(path) => {
                write!(f, "{} is outside the worktree", path.display())
            }
        }
    }
}

/// Why Workspaces could not do what was asked.
#[derive(Debug)]
pub enum WorkspaceError {
    /// The request was refused; the user can mend it.
    Refused(Refusal),
    /// git failed.
    Git(GitError),
    /// The Store failed.
    Store(StoreError),
    /// The id of a worktree could not be read or written.
    Id(PathBuf, std::io::Error),
}

impl fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(refusal) => refusal.fmt(f),
            Self::Git(e) => e.fmt(f),
            Self::Store(e) => e.fmt(f),
            Self::Id(path, e) => write!(f, "the id file {}: {e}", path.display()),
        }
    }
}

impl std::error::Error for WorkspaceError {}

impl From<Refusal> for WorkspaceError {
    fn from(refusal: Refusal) -> Self {
        Self::Refused(refusal)
    }
}

impl From<GitError> for WorkspaceError {
    fn from(e: GitError) -> Self {
        Self::Git(e)
    }
}

impl From<StoreError> for WorkspaceError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

/// The workspaces of the user: the repositories they opened, and every worktree of each.
///
/// What it knows outlives it: a second `Workspaces` on the same Store lists the same
/// repositories, with the same ids and the same bases. It may be used from several
/// threads at once.
///
/// It keeps the worktree of each workspace it was asked about, and with it the one git
/// process that worktree keeps for file contents; a worktree that moved is opened again.
#[derive(Debug)]
pub struct Workspaces {
    git: Git,
    store: Store,
    // The worktrees read so far, kept for their running git process.
    worktrees: Mutex<BTreeMap<WorkspaceId, Arc<Worktree>>>,
    // Held while ids are read, made and kept, so that two Windows that open or list at
    // once do not give one worktree two ids.
    naming: Mutex<()>,
}

impl Workspaces {
    /// The workspaces kept in `store`, read through `git`.
    pub fn new(git: Git, store: Store) -> Workspaces {
        Workspaces {
            git,
            store,
            worktrees: Mutex::new(BTreeMap::new()),
            naming: Mutex::new(()),
        }
    }

    /// Opens the repository that `folder` lies in, and with it the workspace of the
    /// worktree that `folder` lies in. Opening what is open already changes nothing and
    /// gives the same ids.
    ///
    /// The folder may be the worktree itself or any folder inside it. Refused, with the
    /// reason, are a path that is no folder, a folder in no repository, and a repository
    /// with no files checked out.
    ///
    /// An id the Store already holds at another path:
    /// - is the same workspace, moved, when that path no longer holds a worktree with
    ///   this id: the Store is brought up to date;
    /// - belongs to a copy when the other path still holds it. A copy of a whole
    ///   repository has a git folder of its own: it is another repository, and its
    ///   workspace gets a new id, written over the copied one. A copy of one linked
    ///   worktree shares the git folder of the original, so git lists one worktree for
    ///   the two folders: it is refused with [`Refusal::CopiedWorktree`].
    ///
    /// The user is not asked; that question comes when there are reviews to lose.
    pub fn open_folder(&self, folder: &Path) -> Result<Opened, WorkspaceError> {
        let worktree = Worktree::open(&self.git, folder).map_err(|e| match e {
            OpenError::NoSuchFolder => Refusal::NoSuchFolder(folder.to_owned()).into(),
            OpenError::NotARepository => Refusal::NotARepository(folder.to_owned()).into(),
            OpenError::Bare => Refusal::BareRepository(folder.to_owned()).into(),
            OpenError::Git(e) => WorkspaceError::Git(e),
        })?;
        let _naming = lock(&self.naming);

        // git knows one folder for each git folder of a worktree. When it is another
        // folder than this one, and still there, this one is a copy of it. When it is
        // gone, this is the worktree itself, moved without git's knowing.
        let listed = worktree.list()?;
        if listed.iter().all(|other| other.path != worktree.root())
            && listed
                .iter()
                .any(|other| other.git_dir == worktree.git_dir())
        {
            return Err(Refusal::CopiedWorktree(folder.to_owned()).into());
        }

        let kept = self.store.workspaces()?;
        let here = identity::read(worktree.git_dir())?;
        let known = here
            .as_ref()
            .and_then(|id| kept.iter().find(|record| &record.id == id));
        let moved = known.filter(|record| record.path != worktree.root());
        let workspace = match (here, moved) {
            // The id is at home in another folder, which still holds it: this folder
            // came to it by a copy of a whole repository.
            (_, Some(elsewhere)) if self.still_holds(elsewhere) => {
                identity::give(worktree.git_dir())?
            }
            (Some(id), _) => id,
            (None, _) => identity::give(worktree.git_dir())?,
        };

        let repositories = self.store.repositories()?;
        let at_home = repositories
            .iter()
            .find(|record| record.git_dir == worktree.common_dir());
        let repository = match (at_home, moved) {
            (Some(record), _) => record.id.clone(),
            // A workspace that moved, in a repository that is not known at this place:
            // the repository moved with it.
            (None, Some(before)) if before.id == workspace => before.repository.clone(),
            (None, _) => RepositoryId::new(identity::random()?),
        };
        self.store.put_repository(&RepositoryRecord {
            id: repository.clone(),
            git_dir: worktree.common_dir().to_owned(),
        })?;
        self.store
            .put_workspace(&workspace, &repository, worktree.root())?;
        lock(&self.worktrees).insert(workspace.clone(), Arc::new(worktree));
        Ok(Opened {
            repository,
            workspace,
        })
    }

    /// Every opened repository, in the order they were opened, each with the worktrees
    /// git lists for it now. Every worktree is a workspace: one seen for the first time
    /// gets its id here. A repository whose folder is gone is listed with no worktree.
    ///
    /// So is a repository that cannot be read now, because git fails in it or the id of
    /// one of its worktrees can be neither read nor written: the others are listed all
    /// the same. What is wrong there, a call that names one of its workspaces says.
    ///
    /// A worktree that was opened after its folder was moved without git's knowing is
    /// listed too, after the others, for as long as git has not caught up.
    ///
    /// A worktree that is gone, removed by git or deleted, is listed, after the others of
    /// its repository and with no head, for as long as one of its reviews in the Store
    /// holds a thread: its reviews are kept until the user deletes them, and they are not
    /// to be lost from sight. A worktree that is gone and holds no thread is not listed,
    /// as before. A call that needs the files of a worktree that is gone is refused with
    /// [`Refusal::UnknownWorkspace`], as for an id nobody knows.
    pub fn repositories(&self) -> Result<Vec<Repository>, WorkspaceError> {
        let _naming = lock(&self.naming);
        let kept = self.store.workspaces()?;
        let mut repositories = Vec::new();
        for record in self.store.repositories()? {
            let workspaces = match self.worktrees_of(&record, &kept) {
                Ok(workspaces) => workspaces,
                // `Repository` has no place yet to say what failed.
                Err(e) if of_one_repository(&e) => Vec::new(),
                Err(e) => return Err(e),
            };
            repositories.push(Repository {
                id: record.id,
                name: name_of(&record.git_dir),
                git_dir: record.git_dir,
                workspaces,
            });
        }
        Ok(repositories)
    }

    /// The worktrees of one kept repository as they are now, each kept as a workspace;
    /// `kept` is what the Store held of all repositories before.
    fn worktrees_of(
        &self,
        record: &RepositoryRecord,
        kept: &[WorkspaceRecord],
    ) -> Result<Vec<Workspace>, WorkspaceError> {
        let mut workspaces = Vec::new();
        for listed in Worktree::list_of(&self.git, &record.git_dir)? {
            let id = match identity::read(&listed.git_dir)? {
                Some(id) => id,
                None => identity::give(&listed.git_dir)?,
            };
            self.store.put_workspace(&id, &record.id, &listed.path)?;
            workspaces.push(Workspace {
                id,
                path: listed.path,
                head: Some(listed.head),
            });
        }
        // What the Store holds for this repository and git did not list: gone, as a
        // rule, and then passed over; or moved by hand and opened since.
        for known in kept.iter().filter(|known| known.repository == record.id) {
            if workspaces.iter().any(|listed| listed.id == known.id) {
                continue;
            }
            match self.worktree_of(known) {
                Ok(worktree) if worktree.common_dir() == record.git_dir => {
                    workspaces.push(Workspace {
                        id: known.id.clone(),
                        path: known.path.clone(),
                        head: Some(worktree.head()?),
                    });
                }
                Ok(_) | Err(WorkspaceError::Refused(Refusal::UnknownWorkspace(_))) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(workspaces)
    }

    /// What the changes of a workspace are counted from: the base the user set, else the
    /// default branch of the repository by [`default_base`], else nothing. A base the
    /// user set is returned as it is kept, whether or not git still knows it.
    pub fn base(&self, workspace: &WorkspaceId) -> Result<Option<Base>, WorkspaceError> {
        let (record, worktree) = self.workspace(workspace)?;
        Ok(base_of(&record, &worktree.head()?, &worktree.refs()?))
    }

    /// Sets the base of a workspace to a branch or a commit, as the user wrote it. It
    /// replaces the default from now on, and a later `Workspaces` on the same Store still
    /// finds it. Refused when git knows nothing by that name.
    ///
    /// What is kept, and returned, depends on what the text names. A branch, local or of
    /// a remote, is kept by its short name and followed: the base moves when the branch
    /// does. Anything else, a tag, an id cut short, `HEAD~3`, is kept as the full id of
    /// the commit it names now, so that the base stays where the user put it.
    pub fn set_base(
        &self,
        workspace: &WorkspaceId,
        revision: &str,
    ) -> Result<Base, WorkspaceError> {
        let (_, worktree) = self.workspace(workspace)?;
        let name = match branch_named(revision, &worktree.refs()?) {
            Some(branch) => branch,
            None => worktree
                .resolve(revision)?
                .ok_or_else(|| Refusal::UnknownRevision(revision.to_owned()))?
                .to_string(),
        };
        self.store.set_base(workspace, &name)?;
        Ok(Base {
            name,
            source: BaseSource::User,
        })
    }

    /// The changes of a workspace as they are now; see [`Changes`] for what they hold in
    /// each case. The comparison is the first of these that applies:
    ///
    /// 1. there is no base: `NothingToCompare`, [`NoComparison::NoBase`];
    /// 2. the base, which then is one the user set, is a branch that is gone or a commit
    ///    git does not know: `NothingToCompare`, [`NoComparison::BaseGone`];
    /// 3. the workspace is on its base branch, by [`on_base_branch`]: `NotOnUpstream`
    ///    when the branch checked out follows a branch that exists, else
    ///    `OnNoRemoteBranch` when the repository has a remote, else `NothingToCompare`,
    ///    [`NoComparison::NoRemote`];
    /// 4. what is checked out shares a commit with the base: `SinceBase`;
    /// 5. otherwise `NothingToCompare`, [`NoComparison::NoSharedCommit`].
    ///
    /// The base and the upstream are read as the branches they are. A tag with the name
    /// of either, which git would take first, counts for nothing here.
    pub fn changes(&self, workspace: &WorkspaceId) -> Result<Changes, WorkspaceError> {
        let (record, worktree) = self.workspace(workspace)?;
        let head = worktree.head()?;
        let refs = worktree.refs()?;
        let checked_out = match &head {
            Head::Branch { commit, .. } => commit.clone(),
            Head::Detached(commit) => Some(commit.clone()),
        };
        // In every case but one the files are what is not committed, and no commit is
        // listed unless the case names some.
        let uncommitted = |comparison: Comparison, commits: Vec<Commit>| {
            Ok(Changes {
                comparison,
                files: worktree.changed_paths(checked_out.as_ref())?,
                counted_from: checked_out.clone(),
                commits,
            })
        };
        let nothing = |base: Option<Base>, why: NoComparison| {
            uncommitted(Comparison::NothingToCompare { base, why }, Vec::new())
        };

        let Some(base) = base_of(&record, &head, &refs) else {
            return nothing(None, NoComparison::NoBase);
        };
        let Some(base_commit) = commit_of_base(&worktree, &base.name, &refs)? else {
            return nothing(Some(base), NoComparison::BaseGone);
        };
        if on_base_branch(&head, &base.name, &refs) {
            let upstream = match &head {
                Head::Branch { name, .. } => refs
                    .local
                    .iter()
                    .find(|branch| &branch.name == name)
                    .and_then(|branch| branch.upstream.clone()),
                Head::Detached(_) => None,
            };
            return match upstream {
                Some(upstream) => {
                    let commits = worktree.commits(Commits::NotOn(followed(&upstream, &refs)))?;
                    uncommitted(Comparison::NotOnUpstream { base, upstream }, commits)
                }
                None if !refs.remotes.is_empty() => {
                    let commits = worktree.commits(Commits::OnNoRemoteBranch)?;
                    uncommitted(Comparison::OnNoRemoteBranch { base }, commits)
                }
                None => nothing(Some(base), NoComparison::NoRemote),
            };
        }
        match worktree.fork_point(&base_commit)? {
            Some(fork) => Ok(Changes {
                comparison: Comparison::SinceBase { base },
                commits: worktree.commits(Commits::Since(&fork))?,
                files: worktree.changed_paths(Some(&fork))?,
                counted_from: Some(fork),
            }),
            None => nothing(Some(base), NoComparison::NoSharedCommit),
        }
    }

    /// The two versions of one file of a workspace: the old one in the commit `since`, at
    /// `old_path`, and the new one on disk, at `path`. Both paths are relative to the
    /// folder of the worktree. The caller takes `since` from [`Changes::counted_from`]
    /// and, for a renamed file, `old_path` from the path it had; each version is one
    /// read, and nothing about the base or the branch is asked again.
    ///
    /// With `since` none the old version is none. Refused when a path leads out of the
    /// worktree, and when git knows no commit `since`, which has to be the full id of
    /// one.
    pub fn file(
        &self,
        workspace: &WorkspaceId,
        since: Option<&CommitId>,
        old_path: &Path,
        path: &Path,
    ) -> Result<FileSides, WorkspaceError> {
        for asked in [old_path, path] {
            if leads_out(asked) {
                return Err(Refusal::OutsideWorkspace(asked.to_owned()).into());
            }
        }
        let (_, worktree) = self.workspace(workspace)?;
        let old = match since {
            Some(since) => {
                let old = worktree.content_at(since, old_path)?;
                // No content is the answer for a file the commit lacks and for a commit
                // git lacks; only then is it worth a second question to tell them apart.
                if old.is_none() && worktree.resolve(since.as_str())?.as_ref() != Some(since) {
                    return Err(Refusal::UnknownRevision(since.to_string()).into());
                }
                old
            }
            None => None,
        };
        Ok(FileSides {
            old,
            new: worktree.content_on_disk(path)?,
        })
    }

    /// The worktree of a workspace, as the Recorder reads and records it: what the other
    /// parts of the Keeper are handed to read, record and commit in it. The one kept from
    /// an earlier call, with its running git process, when it is still the worktree at
    /// that folder. None for a workspace the Store holds whose worktree is gone: its
    /// folder is gone or holds another worktree. Refused with
    /// [`Refusal::UnknownWorkspace`] for an id the Store does not hold.
    ///
    /// It reads the Store, the folder and the worktree's id file, and starts no git
    /// beyond what opening a worktree not seen before takes: a request may call it every
    /// time, where [`Workspaces::repositories`] runs git in every repository.
    pub fn worktree(
        &self,
        workspace: &WorkspaceId,
    ) -> Result<Option<Arc<Worktree>>, WorkspaceError> {
        let _ = workspace;
        todo!()
    }

    /// What the Store holds for a workspace, and its worktree.
    fn workspace(
        &self,
        id: &WorkspaceId,
    ) -> Result<(WorkspaceRecord, Arc<Worktree>), WorkspaceError> {
        let record = self
            .store
            .workspaces()?
            .into_iter()
            .find(|record| &record.id == id)
            .ok_or_else(|| Refusal::UnknownWorkspace(id.clone()))?;
        let worktree = self.worktree_of(&record)?;
        Ok((record, worktree))
    }

    /// The worktree of a workspace the Store holds: the one kept from an earlier call
    /// when it is still the worktree at that folder, else opened now and kept. A folder
    /// that is gone, or that no longer holds the worktree with this id, is no workspace.
    fn worktree_of(&self, record: &WorkspaceRecord) -> Result<Arc<Worktree>, WorkspaceError> {
        let unknown = || {
            lock(&self.worktrees).remove(&record.id);
            WorkspaceError::Refused(Refusal::UnknownWorkspace(record.id.clone()))
        };
        if !record.path.is_dir() {
            return Err(unknown());
        }
        let kept = lock(&self.worktrees)
            .get(&record.id)
            .filter(|worktree| worktree.root() == record.path)
            .cloned();
        let worktree = match kept {
            Some(worktree) => worktree,
            None => match Worktree::open(&self.git, &record.path) {
                Ok(worktree) if worktree.root() == record.path => Arc::new(worktree),
                Ok(_)
                | Err(OpenError::NoSuchFolder | OpenError::NotARepository | OpenError::Bare) => {
                    return Err(unknown());
                }
                Err(OpenError::Git(e)) => return Err(e.into()),
            },
        };
        if identity::read(worktree.git_dir())?.as_ref() != Some(&record.id) {
            return Err(unknown());
        }
        lock(&self.worktrees).insert(record.id.clone(), Arc::clone(&worktree));
        Ok(worktree)
    }

    /// Whether the folder the Store names for a workspace still holds the worktree with
    /// its id.
    fn still_holds(&self, record: &WorkspaceRecord) -> bool {
        Worktree::open(&self.git, &record.path).is_ok_and(|worktree| {
            worktree.root() == record.path
                && identity::read(worktree.git_dir())
                    .is_ok_and(|id| id.as_ref() == Some(&record.id))
        })
    }
}

/// Whether a failure is that of one repository and leaves the others as readable as they
/// were: git ran there and failed, or said what is not understood, or the id of a
/// worktree could not be read or written. That git cannot be run at all, or that the
/// Store fails, is the failure of no repository in particular.
fn of_one_repository(error: &WorkspaceError) -> bool {
    matches!(
        error,
        WorkspaceError::Git(GitError::Failed { .. } | GitError::Unreadable { .. })
            | WorkspaceError::Id(..)
    )
}

/// The base of a workspace: what the user set, else what the rule finds.
fn base_of(record: &WorkspaceRecord, head: &Head, refs: &Refs) -> Option<Base> {
    match &record.base {
        Some(name) => Some(Base {
            name: name.clone(),
            source: BaseSource::User,
        }),
        None => default_base(head, refs).map(|name| Base {
            name,
            source: BaseSource::Default,
        }),
    }
}

/// The commit a base names now; none when it names none any more. A base is the short
/// name of a branch or the full id of a commit, and is read as nothing else: where the
/// branch is gone, a tag of its name is not the base.
fn commit_of_base(
    worktree: &Worktree,
    base: &str,
    refs: &Refs,
) -> Result<Option<CommitId>, GitError> {
    // A local branch before one of a remote with the same short name: the order in
    // which git itself reads a short name, and that of `branch_named`.
    let local = refs.local.iter().any(|branch| branch.name == base);
    let remote = refs.remote.iter().any(|branch| branch.name == base);
    if local {
        worktree.commit_of(Branch::Local(base))
    } else if remote {
        worktree.commit_of(Branch::Remote(base))
    } else {
        // git reads a full id as that commit and as nothing else. Whatever else it finds
        // under this text is another commit than the text says.
        let named = worktree.resolve(base)?;
        Ok(named.filter(|commit| commit.as_str() == base))
    }
}

/// The branch that the branch checked out follows, from its short name in `refs`, which
/// lists it only while it exists. It is a branch of a remote unless the only branch of
/// that name is a local one; the default a remote recorded, which can be followed too,
/// is among the names of the remote.
fn followed<'a>(upstream: &'a str, refs: &Refs) -> Branch<'a> {
    let local = refs.local.iter().any(|branch| branch.name == upstream);
    let remote = refs.remote.iter().any(|branch| branch.name == upstream);
    if local && !remote {
        Branch::Local(upstream)
    } else {
        Branch::Remote(upstream)
    }
}

/// The short name of the branch that `text` names, however the user wrote it: by its
/// short name or by one of git's longer ways to say local or remote. None when it names
/// no branch.
fn branch_named(text: &str, refs: &Refs) -> Option<String> {
    let local = |name: &str| refs.local.iter().any(|branch| branch.name == name);
    let remote = |name: &str| refs.remote.iter().any(|branch| branch.name == name);
    let stripped =
        |prefixes: [&str; 2]| prefixes.iter().find_map(|prefix| text.strip_prefix(prefix));
    let name = if local(text) || remote(text) {
        text
    } else if let Some(name) = stripped(["refs/heads/", "heads/"]).filter(|name| local(name)) {
        name
    } else {
        stripped(["refs/remotes/", "remotes/"]).filter(|name| remote(name))?
    };
    Some(name.to_owned())
}

/// Whether a path, meant to be counted from the folder of a worktree, leads out of it
/// by its own words.
fn leads_out(path: &Path) -> bool {
    path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
}

/// The name to show for a repository: that of the folder that holds it, which is the
/// folder above its git folder, or the git folder itself where no files are checked out
/// beside it.
fn name_of(git_dir: &Path) -> String {
    let folder = match git_dir.file_name() {
        Some(name) if name == ".git" => git_dir.parent().and_then(Path::file_name),
        name => name,
    };
    folder
        .unwrap_or(git_dir.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// Takes a lock, also after a thread that held it panicked: what it guards is brought
/// up to date by every call that uses it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

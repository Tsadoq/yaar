//! Workspaces (`H-WORKSPACE`).
//!
//! Owns: workspaces, worktree identity, base, creating worktrees, the worktree watcher, who is credited with a changed path.
//!
//! Surface:
//! - [`Workspaces`]: opening a folder, the repositories with their worktrees, the base of
//!   a workspace and setting it, the changes of a workspace and the two versions of one
//!   of its files;
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

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use yaar_recorder::{Git, GitError, Worktree};
use yaar_store::{Store, StoreError};

pub use base::{default_base, on_base_branch};
pub use yaar_recorder::{Change, ChangedPath, Commit, CommitId, Head, Refs};
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
    /// The folder of the worktree.
    pub path: PathBuf,
    /// What is checked out in it.
    pub head: Head,
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
    /// The base the user set names nothing git knows any more.
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
/// It keeps the worktree of each workspace it has read, and with it the one git process
/// that worktree keeps for file contents; a worktree that moved is opened again.
#[derive(Debug)]
pub struct Workspaces {
    git: Git,
    store: Store,
    // The worktrees read so far, kept for their running git process.
    worktrees: Mutex<BTreeMap<WorkspaceId, Arc<Worktree>>>,
}

impl Workspaces {
    /// The workspaces kept in `store`, read through `git`.
    pub fn new(git: Git, store: Store) -> Workspaces {
        todo!()
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
        todo!()
    }

    /// Every opened repository, in the order they were opened, each with the worktrees
    /// git lists for it now. Every worktree is a workspace: one seen for the first time
    /// gets its id here. A repository whose folder is gone is listed with no worktree.
    pub fn repositories(&self) -> Result<Vec<Repository>, WorkspaceError> {
        todo!()
    }

    /// What the changes of a workspace are counted from: the base the user set, else the
    /// default branch of the repository by [`default_base`], else nothing. A base the
    /// user set is returned as it is kept, whether or not git still knows it.
    pub fn base(&self, workspace: &WorkspaceId) -> Result<Option<Base>, WorkspaceError> {
        todo!()
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
        todo!()
    }

    /// The changes of a workspace as they are now; see [`Changes`] for what they hold in
    /// each case. The comparison is the first of these that applies:
    ///
    /// 1. there is no base: `NothingToCompare`, [`NoComparison::NoBase`];
    /// 2. the base, which then is one the user set, names nothing git knows:
    ///    `NothingToCompare`, [`NoComparison::BaseGone`];
    /// 3. the workspace is on its base branch, by [`on_base_branch`]: `NotOnUpstream`
    ///    when the branch checked out follows a branch that exists, else
    ///    `OnNoRemoteBranch` when the repository has a remote, else `NothingToCompare`,
    ///    [`NoComparison::NoRemote`];
    /// 4. what is checked out shares a commit with the base: `SinceBase`;
    /// 5. otherwise `NothingToCompare`, [`NoComparison::NoSharedCommit`].
    pub fn changes(&self, workspace: &WorkspaceId) -> Result<Changes, WorkspaceError> {
        todo!()
    }

    /// The two versions of one file of a workspace: the old one in the commit `since`, at
    /// `old_path`, and the new one on disk, at `path`. Both paths are relative to the
    /// folder of the worktree. The caller takes `since` from [`Changes::counted_from`]
    /// and, for a renamed file, `old_path` from the path it had; each version is one
    /// read, and nothing about the base or the branch is asked again.
    ///
    /// With `since` none the old version is none. Refused when a path leads out of the
    /// worktree, and when git knows no commit `since`.
    pub fn file(
        &self,
        workspace: &WorkspaceId,
        since: Option<&CommitId>,
        old_path: &Path,
        path: &Path,
    ) -> Result<FileSides, WorkspaceError> {
        todo!()
    }
}

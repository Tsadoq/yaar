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
//!   [`Comparison`], [`FileSides`], and [`WorkspaceError`] with the [`Refusal`] a user
//!   can mend.
//!
//! A workspace is one worktree. It is recognised by an id written once into the git
//! folder of that worktree, in a file named [`ID_FILE`]: the id stays with the worktree
//! when the folder is moved, and git lists nothing in that place. Everything else the app
//! keeps about a workspace is in the Store, under that id.
//!
//! This crate starts no git and opens no database: it asks the Recorder and the Store.

mod base;

use std::fmt;
use std::path::{Path, PathBuf};

use yaar_recorder::{Git, GitError};
use yaar_store::{Store, StoreError};

pub use base::{default_base, on_base_branch};
pub use yaar_recorder::{Change, ChangedPath, Commit, CommitId, Head, Refs};

/// The name of the file, in the git folder of a worktree, that holds the id of its
/// workspace: 32 lowercase hexadecimal digits and a line feed.
pub const ID_FILE: &str = "yaar-id";

/// Names one workspace. It is the content of [`ID_FILE`]: 32 lowercase hexadecimal
/// digits, drawn at random when the worktree is first seen.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkspaceId(String);

impl WorkspaceId {
    /// The id with this text.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The text of the id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Names one repository: 32 lowercase hexadecimal digits, drawn at random when the
/// repository is first opened and kept in the Store with the git folder of the repository.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepositoryId(String);

impl RepositoryId {
    /// The id with this text.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The text of the id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

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
    /// The branch or commit, as the user would write it to git.
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
        /// Where the branch left it.
        fork_point: CommitId,
    },
    /// On its base branch, which follows an upstream.
    NotOnUpstream {
        /// The base, which is the branch checked out.
        base: Base,
        /// The branch it follows, by its short name.
        upstream: String,
    },
    /// On its base branch, which follows nothing, in a repository with a remote.
    OnNoRemoteBranch {
        /// The base, which is the branch checked out.
        base: Base,
    },
    /// Nothing to compare with: on the base branch of a repository with no remote, or
    /// with no base found. Whoever shows this offers to set a base.
    NothingToCompare {
        /// The base, when one was found.
        base: Option<Base>,
    },
}

/// The changes of one workspace.
///
/// | [`Comparison`] | `commits` | `files` differ between |
/// | --- | --- | --- |
/// | `SinceBase` | of the branch since the fork point | the fork point and the files on disk |
/// | `NotOnUpstream` | not on the upstream | the checked-out commit and the files on disk |
/// | `OnNoRemoteBranch` | on no branch of any remote | the same |
/// | `NothingToCompare` | none | the same |
///
/// "The files on disk" takes in what is not committed and what git does not track yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changes {
    /// What the workspace is compared with.
    pub comparison: Comparison,
    /// The commits of the comparison, newest first.
    pub commits: Vec<Commit>,
    /// The changed files, ordered by path.
    pub files: Vec<ChangedPath>,
}

/// The two versions of one file of a workspace. A version is none where the file does not
/// exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSides {
    /// The file at the commit the files of [`Changes`] are counted from.
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
            Self::UnknownWorkspace(id) => write!(f, "there is no workspace {}", id.as_str()),
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
#[derive(Debug)]
pub struct Workspaces {
    git: Git,
    store: Store,
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
    /// default branch of the repository by [`default_base`], else nothing.
    pub fn base(&self, workspace: &WorkspaceId) -> Result<Option<Base>, WorkspaceError> {
        todo!()
    }

    /// Sets the base of a workspace to a branch or a commit, as the user wrote it. It
    /// replaces the default from now on, and a later `Workspaces` on the same Store still
    /// finds it. Refused when git knows nothing by that name.
    pub fn set_base(
        &self,
        workspace: &WorkspaceId,
        revision: &str,
    ) -> Result<Base, WorkspaceError> {
        todo!()
    }

    /// The changes of a workspace as they are now; see [`Changes`] for what they hold in
    /// each case. The workspace is on its base branch when [`on_base_branch`] says so.
    pub fn changes(&self, workspace: &WorkspaceId) -> Result<Changes, WorkspaceError> {
        todo!()
    }

    /// The two versions of one file of a workspace, as [`Changes`] compares them: at the
    /// commit its files are counted from, and on disk. For a renamed file, give its path
    /// now; the old version is read at the path it had. Refused when the path leads out
    /// of the worktree.
    pub fn file(&self, workspace: &WorkspaceId, path: &Path) -> Result<FileSides, WorkspaceError> {
        todo!()
    }
}

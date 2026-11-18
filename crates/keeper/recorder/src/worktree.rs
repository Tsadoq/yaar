//! One worktree, read through git.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::Mutex;

use crate::git::{Git, GitError};

/// Names one commit: its full id, in lowercase hexadecimal digits.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommitId(String);

impl CommitId {
    /// The id with these digits.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The digits of the id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CommitId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What is checked out in a worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    /// A branch, by its short name. `commit` is none while the branch has no commit yet,
    /// as in a repository that was just made.
    Branch {
        /// The short name of the branch: `main`, `feature/login`.
        name: String,
        /// The commit the branch points at.
        commit: Option<CommitId>,
    },
    /// No branch: a commit.
    Detached(CommitId),
}

/// One commit, as a list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// Its id.
    pub id: CommitId,
    /// The first line of its message.
    pub summary: String,
}

/// A worktree as its repository lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// The folder of the worktree.
    pub path: PathBuf,
    /// What is checked out in it.
    pub head: Head,
    /// Whether it is the main worktree, the one the repository was made with.
    pub main: bool,
}

/// A local branch, seen from what is checked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalBranch {
    /// Its short name: `main`.
    pub name: String,
    /// How many commits the checked-out commit has that this branch does not: the
    /// distance from the point where the two part to what is checked out.
    pub ahead: u32,
    /// The branch it follows, by its short name: `origin/main`. None also when the
    /// branch it was set to follow is no longer there.
    pub upstream: Option<String>,
}

/// A branch of a remote as the repository last saw it, seen from what is checked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteBranch {
    /// Its short name, the remote first: `origin/main`.
    pub name: String,
    /// The remote it belongs to: `origin`.
    pub remote: String,
    /// How many commits the checked-out commit has that this branch does not.
    pub ahead: u32,
}

/// The branches around a worktree: everything the rule that finds the base needs, read
/// from the repository as it is, with no call to any server.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Refs {
    /// The names of the remotes.
    pub remotes: Vec<String>,
    /// The default branch each remote recorded, by its short name, `origin/main`, in the
    /// order of `remotes`. Listed as recorded, whether or not that branch still exists.
    pub remote_defaults: Vec<String>,
    /// Every local branch, in git's order.
    pub local: Vec<LocalBranch>,
    /// Every branch of every remote, in git's order.
    pub remote: Vec<RemoteBranch>,
}

/// What happened to a path between a commit and the files on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// The file is new. A file git does not track yet, and does not ignore, is new.
    Added,
    /// The content or the kind of the file changed.
    Modified,
    /// The file is gone.
    Deleted,
    /// git pairs the file with one that is gone, by its own rule for renames.
    Renamed {
        /// The path of the file that is gone.
        from: PathBuf,
    },
}

/// One path that differs between a commit and the files on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedPath {
    /// The path now, relative to the folder of the worktree.
    pub path: PathBuf,
    /// What happened to it.
    pub change: Change,
}

/// Which commits to list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Commits<'a> {
    /// Those reachable from what is checked out and not from this commit.
    Since(&'a CommitId),
    /// Those reachable from what is checked out and not from this branch.
    NotOn(&'a str),
    /// Those reachable from what is checked out and from no branch of any remote, as the
    /// repository last saw the remotes.
    OnNoRemoteBranch,
}

/// Why a folder could not be opened as a worktree.
#[derive(Debug)]
pub enum OpenError {
    /// There is no folder at this path, or it cannot be read.
    NoSuchFolder,
    /// The folder lies in no git repository and no worktree.
    NotARepository,
    /// The folder is a repository with no files checked out.
    Bare,
    /// git could not say.
    Git(GitError),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchFolder => write!(f, "there is no such folder"),
            Self::NotARepository => write!(f, "the folder holds no git repository and no worktree"),
            Self::Bare => write!(f, "the repository has no files checked out"),
            Self::Git(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for OpenError {}

impl From<GitError> for OpenError {
    fn from(e: GitError) -> Self {
        Self::Git(e)
    }
}

/// One worktree, read through git.
///
/// It holds no answer of git: every call asks again, so it always tells what is there
/// now. It may be used from several threads at once.
///
/// What it does keep is one running git process that hands out the contents of files in
/// commits, started at the first [`Worktree::content_at`] and asked again at each later
/// one, so that reading many files starts git once. The process ends, and is waited for,
/// when the `Worktree` is dropped. Whoever reads files of a worktree again and again
/// keeps its `Worktree`.
#[derive(Debug)]
pub struct Worktree {
    git: Git,
    root: PathBuf,
    git_dir: PathBuf,
    common_dir: PathBuf,
    // The running process that hands out file contents, once one was asked for.
    contents: Mutex<Option<Child>>,
}

impl Worktree {
    /// Opens the worktree that `folder` lies in: the folder itself or any folder above
    /// it, as git finds it.
    pub fn open(git: &Git, folder: &Path) -> Result<Worktree, OpenError> {
        todo!()
    }

    /// The folder of the worktree, with every link in its path resolved.
    pub fn root(&self) -> &Path {
        todo!()
    }

    /// The git folder of this worktree alone, with every link resolved: `.git` for the
    /// main worktree, a folder inside it for any other. git lists nothing that is put
    /// here, and moves the folder with the worktree.
    pub fn git_dir(&self) -> &Path {
        todo!()
    }

    /// The git folder all worktrees of the repository share, with every link resolved.
    /// Two worktrees belong to one repository when this is the same.
    pub fn common_dir(&self) -> &Path {
        todo!()
    }

    /// What is checked out.
    pub fn head(&self) -> Result<Head, GitError> {
        todo!()
    }

    /// Every worktree of the repository that has its files on disk, this one included:
    /// the main one first, then in git's order. A repository with no files checked out
    /// is not a worktree and is not listed; nor is a worktree whose folder is gone.
    pub fn list(&self) -> Result<Vec<Listed>, GitError> {
        todo!()
    }

    /// The branches around this worktree, each with its distance from what is checked
    /// out.
    pub fn refs(&self) -> Result<Refs, GitError> {
        todo!()
    }

    /// The commit a branch, a tag or a commit id names, as the user would write it to
    /// git; none when git knows nothing by that name.
    pub fn resolve(&self, revision: &str) -> Result<Option<CommitId>, GitError> {
        todo!()
    }

    /// The point where what is checked out left `base`: the newest commit both have.
    /// None when the two share no commit, or when nothing is committed yet.
    pub fn fork_point(&self, base: &CommitId) -> Result<Option<CommitId>, GitError> {
        todo!()
    }

    /// The commits asked for, newest first.
    pub fn commits(&self, which: Commits<'_>) -> Result<Vec<Commit>, GitError> {
        todo!()
    }

    /// Every path that differs between `since` and the files on disk, ordered by path:
    /// what was committed after `since`, what is changed and not committed, staged or
    /// not, and the files git does not track and does not ignore. With `since` none, as
    /// in a repository with no commit, every file on disk that git does not ignore is new.
    ///
    /// The paths and their pairing into renames are those of git's own diff between the
    /// commit and the worktree; the untracked files are those `git status` lists one by
    /// one. A repository inside the worktree is not looked into.
    pub fn changed_paths(&self, since: Option<&CommitId>) -> Result<Vec<ChangedPath>, GitError> {
        todo!()
    }

    /// The content of the file at `path` in `commit`; none when the commit has no file
    /// there, and also when git knows no such commit, which [`Worktree::resolve`] tells
    /// apart. A link is given as the path it points at, as git stores it.
    ///
    /// Answered by the one process this worktree keeps for contents; a process that has
    /// died is started again once.
    pub fn content_at(&self, commit: &CommitId, path: &Path) -> Result<Option<Vec<u8>>, GitError> {
        todo!()
    }

    /// The content of the file at `path` on disk; none when there is no file there. A
    /// link is given as the path it points at, and is not followed. `path` is relative to
    /// the folder of the worktree and must not lead out of it.
    pub fn content_on_disk(&self, path: &Path) -> Result<Option<Vec<u8>>, GitError> {
        todo!()
    }
}

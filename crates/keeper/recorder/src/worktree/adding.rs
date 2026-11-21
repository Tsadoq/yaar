//! A new worktree, made by the user's own git: the one call of this crate, beside the
//! commit control, that changes the user's repository, and only as the user asked.
//!
//! [`Worktree::add_worktree`] runs `git worktree add -b <branch> <folder> <from>` as the
//! user's plain git ([`crate::Git`]'s plain command, as [`Worktree::commit_all`] runs it):
//! with the user's configuration and hooks, in the folder of this worktree, so that the
//! new worktree is an ordinary one git made, of the same repository. Nothing of the app is
//! written into it: the Workspaces part writes the id file into the new worktree's git
//! folder afterwards, as for any worktree it opens.
//!
//! What it checks before git runs, so that a refusal changes nothing:
//!
//! | Check | Refused with |
//! | --- | --- |
//! | `git check-ref-format --branch <branch>` ends with success, and `branch` does not start with `-` | [`AddError::NotABranchName`] |
//! | `refs/heads/<branch>` does not exist (`git rev-parse -q --verify refs/heads/<branch>`) | [`AddError::BranchExists`] |
//! | `from` is a branch: `refs/heads/<from>` or `refs/remotes/<from>` exists, read as a branch, never as a tag, as the base is read | [`AddError::UnknownBranch`] |
//! | nothing is at `folder` (`symlink_metadata` fails with "not found") | [`AddError::FolderExists`] |
//!
//! git is given `from` by its full name (`refs/heads/<from>` or `refs/remotes/<from>`), so a
//! tag of the same name counts for nothing. A remote branch as `from` makes git set it as
//! the upstream of the new branch, as `git worktree add -b` does by the user's own
//! `branch.autoSetupMerge`; nothing here changes that.
//!
//! When git fails, git removes what it made itself; the call then gives
//! [`AddError::Git`] with git's words. [`Worktree::remove_added`] takes a worktree this call
//! made away again, for a caller whose next step failed: `git worktree remove --force
//! <folder>`, then `git update-ref -d refs/heads/<branch> <commit>`, the commit being the one
//! the branch was made at, so that a branch moved since is left alone. Both run as the user's
//! plain git.

use std::fmt;
use std::path::Path;

use crate::git::GitError;
use crate::worktree::{CommitId, Worktree};

/// Why [`Worktree::add_worktree`] made no worktree.
#[derive(Debug)]
pub enum AddError {
    /// git does not take this as the name of a branch.
    NotABranchName(String),
    /// A branch of this name exists already.
    BranchExists(String),
    /// `from` names no branch, local or of a remote.
    UnknownBranch(String),
    /// Something is at the folder already.
    FolderExists(std::path::PathBuf),
    /// git failed, or refused, with its words.
    Git(GitError),
}

impl fmt::Display for AddError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotABranchName(name) => {
                write!(f, "{name:?} is not a name git takes for a branch")
            }
            Self::BranchExists(name) => write!(f, "a branch named {name} exists already"),
            Self::UnknownBranch(name) => write!(f, "there is no branch named {name}"),
            Self::FolderExists(folder) => write!(f, "{} is there already", folder.display()),
            Self::Git(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for AddError {}

impl From<GitError> for AddError {
    fn from(e: GitError) -> Self {
        Self::Git(e)
    }
}

/// A worktree [`Worktree::add_worktree`] made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    /// Its folder, as given.
    pub folder: std::path::PathBuf,
    /// The new branch.
    pub branch: String,
    /// The commit the branch was made at: what `from` named then.
    pub at: CommitId,
}

impl Worktree {
    /// Makes a new worktree of this worktree's repository at `folder`, on a new branch
    /// `branch` made from the branch `from`, with the user's own git, as the module doc
    /// says. `folder` is absolute; its parent folder is made when it is not there.
    pub fn add_worktree(&self, folder: &Path, branch: &str, from: &str) -> Result<Added, AddError> {
        let _ = (folder, branch, from);
        todo!("check, then git worktree add -b, as the module doc says")
    }

    /// Takes away a worktree [`Worktree::add_worktree`] made, and its branch while it still
    /// points at the commit it was made at, as the module doc says.
    pub fn remove_added(&self, added: &Added) -> Result<(), GitError> {
        let _ = added;
        todo!("git worktree remove --force, then delete the branch, as the module doc says")
    }
}

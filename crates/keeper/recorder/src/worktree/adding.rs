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
//! What it checks before git runs, so that a refusal changes nothing, not even the folder
//! the new worktree goes in:
//!
//! | Check | Refused with |
//! | --- | --- |
//! | `git check-ref-format --branch <branch>` ends with success, and `branch` does not start with `-` | [`AddError::NotABranchName`] |
//! | `refs/heads/<branch>` does not exist (`git rev-parse -q --verify refs/heads/<branch>`) | [`AddError::BranchExists`] |
//! | `from` is a branch: `refs/heads/<from>` or `refs/remotes/<from>` exists, read as a branch, never as a tag, as the base is read | [`AddError::UnknownBranch`] |
//! | nothing is at `folder` (`symlink_metadata` fails with "not found") | [`AddError::FolderExists`] |
//!
//! Only once every check has passed is the parent folder of `folder` made, when it is not
//! there (`fs::create_dir_all`), and the call remembers whether it made it. Before git runs
//! it also notes whether the repository's configuration has a section `branch.<branch>`
//! (`git config --get-regexp` of `^branch\.<branch, its dots escaped>\.`, exit code 1 being
//! "none").
//!
//! git is given `from` by its full name (`refs/heads/<from>` or `refs/remotes/<from>`), so a
//! tag of the same name counts for nothing. A remote branch as `from` makes git set it as
//! the upstream of the new branch, as `git worktree add -b` does by the user's own
//! `branch.autoSetupMerge`; nothing here changes that.
//!
//! One worktree is made at a time in a process: the checks and git's run hold one lock of
//! this module (a `static Mutex<()>`), so that two calls for the same branch or folder
//! cannot both pass the checks.
//!
//! # When git fails
//!
//! git can fail after it made something: a `post-checkout` hook of the user that exits with
//! a status other than 0 makes `git worktree add` fail and leaves the worktree and the
//! branch. So after any failure of git the call looks: `git worktree list --porcelain` for a
//! worktree at `folder`, and `git rev-parse -q --verify refs/heads/<branch>`. When either is
//! there, it was made by this call (both were checked absent under the lock), and
//! [`Worktree::remove_added`]'s steps take it away; then the parent folder is removed when
//! this call made it and it is empty (`fs::remove_dir`, never a removal of what is inside).
//! The call then gives [`AddError::Git`] with git's words. Nothing made by this call is left.
//!
//! # Taking a worktree away again
//!
//! [`Worktree::remove_added`] takes a worktree this call made away again, for a caller whose
//! next step failed, in this order, each as the user's plain git:
//! 1. `git worktree remove --force <folder>`;
//! 2. `git update-ref -d refs/heads/<branch> <commit>`, the commit being the one the branch
//!    was made at, so that a branch moved since is left alone;
//! 3. when the branch was deleted and the repository had no section `branch.<branch>`
//!    before the add (as [`Added`] remembers), `git config --remove-section
//!    branch.<branch>`, which takes away the upstream git wrote for a remote `from`; an exit
//!    code saying there is no such section is no failure;
//! 4. the parent folder of `folder`, when the add made it (as [`Added`] remembers) and it is
//!    empty now (`fs::remove_dir`).

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
    /// Whether the add made the parent folder of `folder`.
    pub made_parent: bool,
    /// Whether the repository's configuration had a section `branch.<branch>` before.
    pub had_branch_config: bool,
}

impl Worktree {
    /// Makes a new worktree of this worktree's repository at `folder`, on a new branch
    /// `branch` made from the branch `from`, with the user's own git, as the module doc
    /// says. `folder` is absolute; its parent folder is made, when it is not there, only
    /// once every check has passed.
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

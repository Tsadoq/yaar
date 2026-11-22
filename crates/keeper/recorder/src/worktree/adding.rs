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
//! Before it makes anything the call also notes whether the repository's configuration has
//! a section `branch.<branch>` (`git config --get-regexp` of `^branch\.<branch, its dots
//! escaped>\.`, exit code 1 being "none"). Only once every check and that read have passed
//! is the parent folder of `folder` made, when it is not there (`fs::create_dir`: that one
//! folder, never its parents, so that a missing grandparent fails before git runs with
//! nothing made), and the call remembers whether it made it.
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

use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::git::{GitError, finish};
use crate::worktree::{Branch, CommitId, Worktree};

/// Held from the first check to the end of git's run, so that two worktrees are never made
/// at once by one process.
static ADDING: Mutex<()> = Mutex::new(());

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
    FolderExists(PathBuf),
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

impl From<io::Error> for AddError {
    fn from(e: io::Error) -> Self {
        Self::Git(e.into())
    }
}

/// A worktree [`Worktree::add_worktree`] made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    /// Its folder, as given.
    pub folder: PathBuf,
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
        let _one_at_a_time = hold();

        // What the checks read is read as every read is: no hook, no lock. No name that
        // starts with a dash is one, and none can hold a zero byte.
        if branch.starts_with('-') || branch.contains('\0') {
            return Err(AddError::NotABranchName(branch.to_owned()));
        }
        let named = self.run(["check-ref-format", "--branch", branch])?;
        // `--branch` also expands `@{-1}` to the name of the branch left last and
        // succeeds: what it prints is the name itself only when the name was one.
        if named.code != Some(0) || named.stdout.strip_suffix(b"\n") != Some(branch.as_bytes()) {
            return Err(AddError::NotABranchName(branch.to_owned()));
        }
        if self.ref_exists(&format!("refs/heads/{branch}"))? {
            return Err(AddError::BranchExists(branch.to_owned()));
        }
        let (full, at) = self
            .branch_to_start_from(from)?
            .ok_or_else(|| AddError::UnknownBranch(from.to_owned()))?;
        match fs::symlink_metadata(folder) {
            Ok(_) => return Err(AddError::FolderExists(folder.to_owned())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }

        // The last read, so that no read can fail once something is made.
        let had_branch_config = self.has_branch_section(branch)?;

        // Every check has passed: from here something is made.
        let made_parent = match folder.parent() {
            Some(parent) if fs::symlink_metadata(parent).is_err() => {
                fs::create_dir(parent)?;
                true
            }
            _ => false,
        };
        let added = Added {
            folder: folder.to_owned(),
            branch: branch.to_owned(),
            at,
            made_parent,
            had_branch_config,
        };

        let mut add = self.git.plain(&self.root);
        add.args(["worktree", "add", "-b", branch])
            .arg(folder)
            .arg(&full);
        let failure = match finish(add) {
            Ok(answer) if answer.code == Some(0) => return Ok(added),
            Ok(answer) => answer.failed(),
            Err(e) => e,
        };
        // git can fail after it made the worktree and the branch, as a hook of the user
        // that ends in error does. Both were checked absent under the lock, so what is
        // there now is this call's. What cannot be taken away is not the answer: git's
        // words are.
        let _ = self.take_away(&added, true);
        Err(AddError::Git(failure))
    }

    /// Takes away a worktree [`Worktree::add_worktree`] made, and its branch while it still
    /// points at the commit it was made at, as the module doc says.
    pub fn remove_added(&self, added: &Added) -> Result<(), GitError> {
        let _one_at_a_time = hold();
        self.take_away(added, false)
    }

    /// The steps of [`Worktree::remove_added`]. After a failed add (`after_failure`) the
    /// worktree may not be listed, and it is taken away only when it is.
    fn take_away(&self, added: &Added, after_failure: bool) -> Result<(), GitError> {
        let mut first_failure = None;
        let there = !after_failure || self.lists(&added.folder)?;
        if there {
            let mut remove = self.git.plain(&self.root);
            remove
                .args(["worktree", "remove", "--force"])
                .arg(&added.folder);
            let answer = finish(remove)?;
            if answer.code != Some(0) {
                // A worktree that is still there keeps its branch: a branch deleted under
                // it would leave a worktree on no branch. One that is gone as a folder is
                // no more than a record git prunes by itself.
                if fs::symlink_metadata(&added.folder).is_ok() {
                    return Err(answer.failed());
                }
                first_failure = Some(answer.failed());
            }
        }

        let full = format!("refs/heads/{}", added.branch);
        let deleted = match self.ref_commit(&full)? {
            Some(now) if now == added.at => {
                let mut delete = self.git.plain(&self.root);
                delete.args(["update-ref", "-d", &full, added.at.as_str()]);
                let answer = finish(delete)?;
                if answer.code != Some(0) {
                    return Err(first_failure.unwrap_or_else(|| answer.failed()));
                }
                true
            }
            // Moved since: the user's now.
            Some(_) => false,
            None => true,
        };
        if deleted && !added.had_branch_config && self.has_branch_section(&added.branch)? {
            let mut remove = self.git.plain(&self.root);
            remove
                .args(["config", "--local", "--remove-section"])
                .arg(format!("branch.{}", added.branch));
            let answer = finish(remove)?;
            // No such section is how the add that wrote none is told apart.
            if answer.code != Some(0) && answer.code != Some(128) {
                return Err(first_failure.unwrap_or_else(|| answer.failed()));
            }
        }
        if added.made_parent
            && let Some(parent) = added.folder.parent()
        {
            // Never a removal of what is inside: a folder that holds anything stays.
            let _ = fs::remove_dir(parent);
        }
        first_failure.map_or(Ok(()), Err)
    }

    /// Whether `git worktree list` has a worktree at `folder`, as git prints it or with
    /// every link resolved.
    fn lists(&self, folder: &Path) -> Result<bool, GitError> {
        let answer = self
            .run(["worktree", "list", "--porcelain", "-z"])?
            .printed()?;
        let wanted = resolved(folder);
        Ok(answer
            .split(|byte| *byte == 0)
            .filter_map(|field| field.strip_prefix(b"worktree "))
            .map(|path| Path::new(OsStr::from_bytes(path)))
            .any(|path| path == folder || resolved(path) == wanted))
    }

    /// The full name of the branch `from` names, local first, and the commit it points
    /// at. None when it names no branch. The ref is asked for exactly, so that `main^` or
    /// `main~1` is no branch.
    fn branch_to_start_from(&self, from: &str) -> Result<Option<(String, CommitId)>, GitError> {
        if from.contains('\0') {
            return Ok(None);
        }
        for branch in [Branch::Local(from), Branch::Remote(from)] {
            let full = branch.full_name();
            if self.ref_exists(&full)?
                && let Some(commit) = self.commit_of(branch)?
            {
                return Ok(Some((full, commit)));
            }
        }
        Ok(None)
    }

    /// Whether the full name `full` names a ref of the repository, exactly.
    fn ref_exists(&self, full: &str) -> Result<bool, GitError> {
        let answer = self.run(["show-ref", "--verify", "-q", "--end-of-options", full])?;
        match answer.code {
            Some(0) => Ok(true),
            // 1 for a ref that is not there, and 128 for a name git cannot be asked about.
            Some(1 | 128) => Ok(false),
            _ => Err(answer.failed()),
        }
    }

    /// The commit the ref `full` points at; none when there is no such ref.
    fn ref_commit(&self, full: &str) -> Result<Option<CommitId>, GitError> {
        if !self.ref_exists(full)? {
            return Ok(None);
        }
        self.resolve(full)
    }

    /// Whether the repository's own configuration has a section `branch.<branch>`.
    fn has_branch_section(&self, branch: &str) -> Result<bool, GitError> {
        let mut pattern = String::from("^branch\\.");
        for c in branch.chars() {
            if ".[]\\^$*+?(){}|".contains(c) {
                pattern.push('\\');
            }
            pattern.push(c);
        }
        pattern.push_str("\\.");
        let answer = self.run(["config", "--local", "--get-regexp", &pattern])?;
        match answer.code {
            Some(0) => Ok(true),
            // Exit code 1 is git's word for nothing matching.
            Some(1) => Ok(false),
            _ => Err(answer.failed()),
        }
    }
}

/// The lock of this module.
fn hold() -> MutexGuard<'static, ()> {
    ADDING.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `path` with its links resolved: as far as it exists, with what is below that as it is.
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

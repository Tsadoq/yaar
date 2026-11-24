//! A new worktree made by the user's own git (`git worktree add -b <branch> <folder> <from>`): the
//! user's configuration and hooks apply. The checks run before git, so a refusal changes nothing;
//! git is given the full name of `from`, so a tag never counts.

use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::git::{GitError, finish};
use crate::worktree::{Branch, CommitId, Worktree, list_printed};

/// Held from the first check to the end of git's run, so that two calls for the same branch or
/// folder cannot both pass the checks.
static ADDING: Mutex<()> = Mutex::new(());

#[derive(Debug)]
pub enum AddError {
    NotABranchName(String),
    BranchExists(String),
    UnknownBranch(String),
    FolderExists(PathBuf),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    pub folder: PathBuf,
    pub branch: String,
    pub at: CommitId,
    pub made_parent: bool,
    pub had_branch_config: bool,
}

impl Worktree {
    pub fn add_worktree(&self, folder: &Path, branch: &str, from: &str) -> Result<Added, AddError> {
        let _one_at_a_time = hold();

        if branch.starts_with('-') || branch.contains('\0') {
            return Err(AddError::NotABranchName(branch.to_owned()));
        }
        let named = self.run(["check-ref-format", "--branch", branch])?;
        // `--branch` also expands `@{-1}` to the name of the branch left last and succeeds: what it
        // prints is the name itself only when the name was one.
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

        let had_branch_config = self.has_branch_section(branch)?;

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
        // git can fail after it made the worktree and the branch, as a failing hook of the user
        // does. What cannot be taken away is not the answer: git's words are.
        let _ = self.take_away(&added, true);
        Err(AddError::Git(failure))
    }

    pub fn remove_added(&self, added: &Added) -> Result<(), GitError> {
        let _one_at_a_time = hold();
        self.take_away(added, false)
    }

    /// After a failed add (`after_failure`) the worktree may not be listed, and it is taken away
    /// only when it is.
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
                // A worktree that is still there keeps its branch: a branch deleted under it would
                // leave a worktree on no branch.
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
            // Moved since: the branch is the user's now, and is left.
            Some(_) => false,
            None => true,
        };
        // Only when the branch went and its `branch.<name>` section did not exist before the add:
        // removing it takes away the upstream git wrote.
        if deleted && !added.had_branch_config && self.has_branch_section(&added.branch)? {
            let mut remove = self.git.plain(&self.root);
            remove
                .args(["config", "--local", "--remove-section"])
                .arg(format!("branch.{}", added.branch));
            let answer = finish(remove)?;
            if answer.code != Some(0) && answer.code != Some(128) {
                return Err(first_failure.unwrap_or_else(|| answer.failed()));
            }
        }
        if added.made_parent
            && let Some(parent) = added.folder.parent()
        {
            let _ = fs::remove_dir(parent);
        }
        first_failure.map_or(Ok(()), Err)
    }

    fn lists(&self, folder: &Path) -> Result<bool, GitError> {
        let answer = list_printed(&self.git, |args| self.run(args))?;
        let wanted = resolved(folder);
        Ok(answer
            .split(|byte| *byte == 0)
            .filter_map(|field| field.strip_prefix(b"worktree "))
            .map(|path| Path::new(OsStr::from_bytes(path)))
            .any(|path| path == folder || resolved(path) == wanted))
    }

    /// The ref is asked for exactly, so that `main^` or `main~1` is no branch.
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

    fn ref_exists(&self, full: &str) -> Result<bool, GitError> {
        let answer = self.run(["show-ref", "--verify", "-q", "--end-of-options", full])?;
        match answer.code {
            Some(0) => Ok(true),
            Some(1 | 128) => Ok(false),
            _ => Err(answer.failed()),
        }
    }

    fn ref_commit(&self, full: &str) -> Result<Option<CommitId>, GitError> {
        if !self.ref_exists(full)? {
            return Ok(None);
        }
        self.resolve(full)
    }

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
            Some(1) => Ok(false),
            _ => Err(answer.failed()),
        }
    }
}

fn hold() -> MutexGuard<'static, ()> {
    ADDING.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `path` with its links resolved as far as it exists; the part below that is kept as it is.
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

//! The writing side: the states of a worktree, recorded without a commit.
//!
//! A state holds what git would commit if the user ran `git add -A` and committed: every
//! tracked file and every untracked file git does not ignore, as git's own filters and
//! line-ending rules make them. It is a commit, but in a state store of the app's own, a
//! bare repository in the app's folder, one for each worktree: never in the real
//! repository.
//!
//! How a state is taken, in short. The user's own git runs in the real repository, on a
//! copy of the worktree's list of files made for this one state, and with every object it
//! writes sent to the state store; the real repository's objects are read beside the
//! store's. `git status` names the paths that differ, those paths are staged into the
//! copy, the tree is written, and a commit of it with the app's own identity is made and
//! named by a ref of the store. The store then copies whatever of that state, and of the
//! commit checked out, it does not hold yet, so that every state reads with the real
//! repository deleted.
//!
//! What recording leaves alone: the user's list of files, their refs, branches, stash and
//! reflog, every working file, and the real repository's objects. Every run turns off the
//! hooks as reading does (the hooks folder, each hook the configuration defines, the
//! file-system monitor), and none depends on the user's identity.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::git::{Git, GitError};
use crate::worktree::{ChangedPath, CommitId, Worktree};

/// Names one recorded state: the full id of its commit in the state store.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StateId(String);

impl StateId {
    /// The id with these digits.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The digits of the id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StateId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One recorded state, as the store lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedState {
    /// Its id.
    pub id: StateId,
    /// The full id of its tree: two states hold the same files when their trees are the
    /// same.
    pub tree: String,
    /// The commit that was checked out in the worktree when it was taken; none where
    /// nothing was committed yet.
    pub on: Option<CommitId>,
}

/// Why a state could not be recorded.
#[derive(Debug)]
pub enum RecordError {
    /// git failed, or the store could not be read or written.
    Git(GitError),
    /// The folder of the store cannot be made or written.
    Store {
        /// The folder of the store.
        folder: PathBuf,
        /// What went wrong, as a sentence.
        reason: String,
    },
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git(e) => e.fmt(f),
            Self::Store { folder, reason } => {
                write!(f, "the state store {}: {reason}", folder.display())
            }
        }
    }
}

impl std::error::Error for RecordError {}

impl From<GitError> for RecordError {
    fn from(e: GitError) -> Self {
        Self::Git(e)
    }
}

/// The recorded states of one worktree, in a store of their own.
///
/// It may be used from several threads at once; states of the one worktree are taken one
/// after the other, never two at a time. Reading a state needs nothing of the real
/// repository: not its objects, not its folder.
#[derive(Debug)]
pub struct StateStore {
    git: Git,
    folder: PathBuf,
}

impl StateStore {
    /// The store in `folder`, a bare repository made there when the folder holds none.
    /// The folder lies in the app's folder, never in a worktree or a repository.
    pub fn open(git: &Git, folder: &Path) -> Result<StateStore, RecordError> {
        let _ = (git, folder);
        todo!()
    }

    /// The folder of the store.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Records the state of `worktree` as its files are now, and returns it. When the
    /// tree is that of the newest state, that state is returned and nothing is written:
    /// a state is the files, and the same files are the same state.
    ///
    /// The real repository is left as it was: its list of files, refs, stash, reflog,
    /// objects and working files. No hook of the user runs and none can stop it. A
    /// repository inside the worktree is recorded as the commit it is on, or left out when
    /// it has none.
    pub fn record(&self, worktree: &Worktree) -> Result<RecordedState, RecordError> {
        let _ = (worktree, &self.git);
        todo!()
    }

    /// Copies a commit of the real repository into the store, with its tree, so that its
    /// files read with the real repository gone, as a thread on the old side of a diff
    /// needs. A commit the store holds already costs nothing.
    pub fn keep_commit(&self, worktree: &Worktree, commit: &CommitId) -> Result<(), RecordError> {
        let _ = (worktree, commit);
        todo!()
    }

    /// Every state of the store, the oldest first.
    pub fn states(&self) -> Result<Vec<RecordedState>, GitError> {
        todo!()
    }

    /// Every file of a state or of a kept commit, ordered by the bytes of its path, as
    /// git orders paths. `tree` is a [`StateId`], or the full id of a commit given to
    /// [`StateStore::keep_commit`].
    pub fn paths(&self, tree: &str) -> Result<Vec<PathBuf>, GitError> {
        let _ = tree;
        todo!()
    }

    /// The content of one file of a state or of a kept commit; none when it has no file
    /// there, or when the store holds no such state or commit. A link is given as the path
    /// it points at, as git stores it.
    pub fn content(&self, tree: &str, path: &Path) -> Result<Option<Vec<u8>>, GitError> {
        let _ = (tree, path);
        todo!()
    }

    /// The paths that differ between two states, or kept commits, of the store, ordered by
    /// path, each with what happened to it on the way from `from` to `to`, renames paired
    /// by git's rule.
    pub fn changed_paths(&self, from: &str, to: &str) -> Result<Vec<ChangedPath>, GitError> {
        let _ = (from, to);
        todo!()
    }
}

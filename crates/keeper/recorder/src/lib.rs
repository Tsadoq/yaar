//! Recorder (`H-STATE`).
//!
//! Owns: recording the worktree, listing states, changed paths, file contents, exports of a tree; every call of git.
//!
//! Surface, the reading side:
//! - [`Git`], the one place in the app that starts git;
//! - [`Worktree`], one worktree as git knows it: where it is, what is checked out, the
//!   branches around it, the commits and the changed files between a commit and the files
//!   on disk, and the content of a file on either side;
//! - what those return: [`Head`], [`CommitId`], [`Commit`], [`Refs`], [`ChangedPath`],
//!   [`Listed`].
//!
//! The writing side, which records a state of the worktree, is not built yet: nothing
//! here writes so far.
//!
//! No other crate starts git. Whatever the rest of the Keeper wants to know of a
//! repository it asks here, in the words above; that a branch is a ref and a version of a
//! file a blob stays inside this crate.
//!
//! Reading leaves the repository as it was. Every call made here
//! - runs the user's own git in the user's repository, with the user's configuration, so
//!   that it sees what the user's git sees;
//! - turns off every hook: the hooks folder, each hook the configuration defines, and the
//!   file-system monitor, which is a program the configuration can name;
//! - takes no lock it can do without, so it does not rewrite the index to refresh it;
//! - never asks at a terminal, and reads git's answers in git's own language.
//!
//! File contents of commits come from one git process per worktree that stays running
//! between reads; every other answer is one run of git.

mod contents;
mod git;
mod index;
mod worktree;

pub use git::{Git, GitError};
pub use worktree::{
    Change, ChangedPath, Commit, CommitId, Commits, Head, Listed, LocalBranch, OpenError, Refs,
    RemoteBranch, Worktree,
};

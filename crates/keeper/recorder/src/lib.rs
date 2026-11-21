//! Recorder (`H-STATE`).
//!
//! Owns: recording the worktree, listing states, changed paths, file contents, exports of a tree; every call of git.
//!
//! Surface, the reading side:
//! - [`Git`], the one place in the app that starts git;
//! - [`Worktree`], one worktree as git knows it: where it is, what is checked out, the
//!   other worktrees of its repository, the branches around it, the commits and the
//!   changed files between a commit and the files on disk, and the content of a file on
//!   either side;
//! - what those take and return: [`Head`], [`CommitId`], [`Commit`], [`Refs`], [`Branch`],
//!   [`ChangedPath`], [`Listed`].
//!
//! Surface, the writing side:
//! - [`StateStore`], the recorded states of one worktree, in a bare repository of the
//!   app's own: [`StateStore::record`] records the state of the files as they are, and the
//!   store gives back its [`RecordedState`]s, their files, their contents and the paths
//!   that differ between two of them, with no need of the real repository;
//! - [`Worktree::commit_all`], the commit control: an ordinary commit of every
//!   uncommitted change, made as the user's own git makes it, with [`CommitError`]. A
//!   commit git or a hook refuses leaves the user's list of files as it was before;
//! - [`StateStore::record_reporting`], a recording that leaves out a repository inside
//!   the worktree with no commit and says so, in a [`Recording`];
//!   [`StateStore::compare`] and [`StateStore::content_in`], the paths that differ
//!   between any two of commits, states and the files on disk, and a file in any of them;
//! - what slice 9 adds: the time each state was recorded at, read from a [`Clock`]
//!   ([`StateStore::open_with`], [`StateStore::recorded_times`]); the clean-up of
//!   everything a store holds that the caller does not keep ([`StateStore::clean_up`],
//!   [`Cleaned`]); the room the stores take on disk ([`sizes`], [`StoreSizes`]); and a new
//!   worktree made by the user's own git ([`Worktree::add_worktree`], [`Added`],
//!   [`AddError`], [`Worktree::remove_added`]);
//! - [`history`], the commits and the states of a worktree in the order they happened,
//!   as [`HistoryEntry`]s.
//!
//! No other crate starts git. Whatever the rest of the Keeper wants to know of a
//! repository it asks here, in the words above; that a branch is a ref and a version of a
//! file a blob stays inside this crate.
//!
//! Reading and recording leave the repository as it was. The commit control is the one
//! call that changes it, as the user asked; it runs the user's git as the user would.
//! Every other call made here
//! - runs the user's own git in the user's repository, with the user's configuration, so
//!   that it sees what the user's git sees;
//! - turns off every hook: the hooks folder, each hook the configuration defines, and the
//!   file-system monitor, which is a program the configuration can name;
//! - takes no lock it can do without, and never writes or locks git's own list of the
//!   files: where git would bring that list up to date, it is given a copy to work on;
//! - is told its repository by the folder alone, not by a variable of the environment
//!   this process happens to have;
//! - never asks at a terminal, and reads git's answers in git's own language.
//!
//! File contents of commits come from one git process per worktree, and one per state
//! store, that stays running between reads and ends with its owner; every other answer is
//! one run of git.

mod clock;
mod contents;
mod git;
mod history;
mod index;
mod states;
mod worktree;

pub use clock::{Clock, SystemClock};
pub use git::{Git, GitError};
pub use history::{Entry as HistoryEntry, history};
pub use states::{
    Cleaned, RecordError, RecordedState, Recording, StateId, StateStore, StoreSizes, sizes,
};
pub use worktree::{
    AddError, Added, Branch, Change, ChangedPath, Commit, CommitError, CommitId, Commits, Head,
    Listed, LocalBranch, OpenError, Refs, RemoteBranch, Worktree,
};

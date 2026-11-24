//! Recorder: the only crate that starts git. Reads and recordings run it with hooks off, on a copy
//! of the index, never asking at a terminal. `Worktree::commit_all`, `add_worktree` and
//! `remove_added` run the user's own git with its hooks.

mod clock;
mod contents;
mod git;
mod history;
mod index;
mod states;
mod worktree;

pub use clock::{Clock, SystemClock};
pub use git::{Git, GitEnvironment, GitError};
pub use history::{Entry as HistoryEntry, history};
pub use states::{
    Deleted, RecordError, RecordedState, Recording, SAME_STATE_FOR, StateId, StateStore,
    StoreSizes, sizes,
};
pub use worktree::{
    AddError, Added, Branch, Change, ChangedPath, Commit, CommitError, CommitId, Commits, Head,
    Listed, LocalBranch, OpenError, Refs, RemoteBranch, Worktree, resolved,
};

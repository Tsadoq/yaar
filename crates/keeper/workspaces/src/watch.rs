//! The watcher of the worktrees: the system's notices of changed files, one watcher for
//! each workspace watched, through the `notify` crate.

/// What the watcher of a worktree saw change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Seen {
    /// Files of the worktree: what is uncommitted may have changed.
    Files,
    /// What is checked out, the list of files or a ref: the name of the workspace, its
    /// commits and what is uncommitted may have changed.
    Head,
}

/// The watchers of the workspaces watched and the ledger of each workspace.
#[derive(Debug, Default)]
pub(crate) struct Watching {
    _todo: (),
}

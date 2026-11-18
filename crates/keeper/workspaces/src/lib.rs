//! Workspaces (`H-WORKSPACE`).
//!
//! Owns: workspaces, worktree identity, base, creating worktrees, the worktree watcher, who is credited with a changed path.
//!
//! Surface: none yet. This crate is a shell: it marks where the component lives so that
//! the layout rules of `cargo xtask layout` guard it from the first commit.

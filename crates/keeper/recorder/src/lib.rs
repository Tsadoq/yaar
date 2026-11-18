//! Recorder (`H-STATE`).
//!
//! Owns: recording the worktree, listing states, changed paths, file contents, exports of a tree; every call of git.
//!
//! Surface: none yet. This crate is a shell: it marks where the component lives so that
//! the layout rules of `cargo xtask layout` guard it from the first commit.
